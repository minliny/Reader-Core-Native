//! P3 residual host-routing parity tests.
//!
//! These cover `java.*` host-routed methods that the in-flight
//! `codex/reader-js-compat-runtime` work does NOT yet bind:
//! `downloadFile`, `cacheFile`, `importScript`, `setContent`, `put`,
//! `reGetBook`. They route through the same `HostCallbackRegistry` /
//! `HostCall` descriptor boundary as `java.ajax` etc., but are wired via a
//! dedicated installer so the existing `HostMethod` enum is untouched
//! (concurrent-agent safety).
//!
//! Fixture shapes mirror legado semantics:
//!   downloadFile(url)        -> relative cache path string
//!   cacheFile(url[, saveTtl])-> cached content text
//!   importScript(path)       -> script text
//!   setContent(content)      -> null (host owns display state)
//!   put(key, value)          -> the value (legado returns the stored value)
//!   reGetBook()              -> null (triggers host re-search; no JS return)

use reader_js::{HostCallbackRegistry, JsErrorKind, JsRuntimeConfig, JsSandbox, QuickJsSandbox};
use serde_json::json;
use std::sync::{Arc, Mutex};

/// Capture every `HostCall` descriptor the host receives, plus a per-name
/// canned response so tests can assert both the descriptor contract and the
/// realistic fixture shape.
fn observing_registry(
    sink: Arc<Mutex<Vec<(String, Vec<serde_json::Value>)>>>,
) -> HostCallbackRegistry {
    let mut registry = HostCallbackRegistry::new();
    let s = Arc::clone(&sink);
    registry.register("java.downloadFile", move |call| {
        s.lock()
            .unwrap()
            .push((call.name.clone(), call.args.clone()));
        Ok(json!("cache/a1b2c3d4e5f6a1b2.html"))
    });
    let s = Arc::clone(&sink);
    registry.register("java.cacheFile", move |call| {
        s.lock()
            .unwrap()
            .push((call.name.clone(), call.args.clone()));
        Ok(json!("cached script body\n"))
    });
    let s = Arc::clone(&sink);
    registry.register("java.importScript", move |call| {
        s.lock()
            .unwrap()
            .push((call.name.clone(), call.args.clone()));
        Ok(json!("function calc(){return 1+1;}\n"))
    });
    let s = Arc::clone(&sink);
    registry.register("java.setContent", move |call| {
        s.lock()
            .unwrap()
            .push((call.name.clone(), call.args.clone()));
        Ok(json!(null))
    });
    let s = Arc::clone(&sink);
    registry.register("java.put", move |call| {
        s.lock()
            .unwrap()
            .push((call.name.clone(), call.args.clone()));
        // legado: java.put returns the stored value.
        Ok(call.args.get(1).cloned().unwrap_or(json!(null)))
    });
    let s = Arc::clone(&sink);
    registry.register("java.reGetBook", move |call| {
        s.lock()
            .unwrap()
            .push((call.name.clone(), call.args.clone()));
        Ok(json!(null))
    });
    registry
}

#[test]
fn download_file_routes_url_descriptor_to_host() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.downloadFile("https://example.test/book/1.html",{type:"html"})"#)
        .unwrap();

    // legado returns a relative cache path: md5_16(url).{type}
    assert_eq!(result.value, json!("cache/a1b2c3d4e5f6a1b2.html"));

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "java.downloadFile");
    assert_eq!(calls[0].1[0], json!("https://example.test/book/1.html"));
    // request-config object (legado inline `,{type=...}`) passes through as 2nd arg.
    assert_eq!(calls[0].1[1], json!({"type": "html"}));
}

#[test]
fn download_file_global_alias_routes_identically() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"downloadFile("https://example.test/file.zip")"#)
        .unwrap();

    assert_eq!(result.value, json!("cache/a1b2c3d4e5f6a1b2.html"));
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, "java.downloadFile");
}

#[test]
fn cache_file_single_arg_routes_descriptor() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.cacheFile("https://example.test/lib.js")"#)
        .unwrap();

    assert_eq!(result.value, json!("cached script body\n"));
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, "java.cacheFile");
    assert_eq!(calls[0].1, vec![json!("https://example.test/lib.js")]);
}

#[test]
fn cache_file_optional_save_time_passes_through() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.cacheFile("https://example.test/lib.js", 3600)"#)
        .unwrap();

    assert_eq!(result.value, json!("cached script body\n"));
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].1[0], json!("https://example.test/lib.js"));
    assert_eq!(calls[0].1[1], json!(3600));
}

#[test]
fn import_script_returns_script_text_from_host() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.importScript("https://example.test/helpers.js")"#)
        .unwrap();

    assert_eq!(result.value, json!("function calc(){return 1+1;}\n"));
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, "java.importScript");
    assert_eq!(calls[0].1, vec![json!("https://example.test/helpers.js")]);
}

#[test]
fn set_content_routes_content_and_optional_base_url() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let r1 = sandbox
        .evaluate(r#"java.setContent("<p>chapter</p>")"#)
        .unwrap();
    assert!(r1.value.is_null());

    let r2 = sandbox
        .evaluate(r#"java.setContent("<p>chapter</p>", "https://example.test/book/1")"#)
        .unwrap();
    assert!(r2.value.is_null());

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "java.setContent");
    assert_eq!(calls[0].1, vec![json!("<p>chapter</p>")]);
    assert_eq!(calls[1].0, "java.setContent");
    assert_eq!(calls[1].1[0], json!("<p>chapter</p>"));
    assert_eq!(calls[1].1[1], json!("https://example.test/book/1"));
}

#[test]
fn put_variable_routes_and_returns_value() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.put("lastChapter", "ch-42")"#)
        .unwrap();

    // legado: java.put returns the stored value.
    assert_eq!(result.value, json!("ch-42"));
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, "java.put");
    assert_eq!(calls[0].1, vec![json!("lastChapter"), json!("ch-42")]);
}

#[test]
fn re_get_book_routes_with_no_args() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox.evaluate(r#"java.reGetBook()"#).unwrap();
    assert!(result.value.is_null());

    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, "java.reGetBook");
    assert!(
        calls[0].1.is_empty(),
        "reGetBook must carry no descriptor args"
    );
}

#[test]
fn residual_methods_fail_closed_when_host_unregistered() {
    // Empty registry: no callback registered for any residual method.
    let registry = HostCallbackRegistry::new();
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let err = sandbox
        .evaluate(r#"java.downloadFile("https://example.test/x")"#)
        .unwrap_err();
    // Message starts with "host callback " -> mapped to HostCallback kind.
    assert_eq!(err.kind, JsErrorKind::HostCallback);
    assert!(err.message.contains("java.downloadFile"));
}

#[test]
fn download_file_rejects_missing_url_argument() {
    let registry = observing_registry(Arc::new(Mutex::new(Vec::new())));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let err = sandbox.evaluate(r#"java.downloadFile()"#).unwrap_err();
    assert_eq!(err.kind, JsErrorKind::Exception);
    assert!(err.message.contains("java.downloadFile"));
    assert!(err.message.contains("URL"));
}

#[test]
fn download_file_rejects_non_string_url() {
    let registry = observing_registry(Arc::new(Mutex::new(Vec::new())));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let err = sandbox.evaluate(r#"java.downloadFile(12345)"#).unwrap_err();
    assert_eq!(err.kind, JsErrorKind::Exception);
}

#[test]
fn put_rejects_missing_value_argument() {
    let registry = observing_registry(Arc::new(Mutex::new(Vec::new())));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let err = sandbox.evaluate(r#"java.put("only-key")"#).unwrap_err();
    assert_eq!(err.kind, JsErrorKind::Exception);
    assert!(err.message.contains("java.put"));
}

#[test]
fn put_rejects_non_string_arguments() {
    let registry = observing_registry(Arc::new(Mutex::new(Vec::new())));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let err = sandbox.evaluate(r#"java.put("key", 12345)"#).unwrap_err();
    assert_eq!(err.kind, JsErrorKind::Exception);
}

#[test]
fn set_content_rejects_non_string_content() {
    let registry = observing_registry(Arc::new(Mutex::new(Vec::new())));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let err = sandbox.evaluate(r#"java.setContent(12345)"#).unwrap_err();
    assert_eq!(err.kind, JsErrorKind::Exception);
}

#[test]
fn residual_descriptor_carries_realistic_response_shapes() {
    // R6 contract: the host receives a well-typed HostCall (name + args) and
    // returns realistic fixture shapes that mirror legado's return semantics,
    // not a generic {"body":"<p>stubbed</p>"} blob.
    let calls = Arc::new(Mutex::new(Vec::new()));

    let cases: &[(&str, serde_json::Value)] = &[
        (
            r#"java.downloadFile("https://example.test/b.html",{type:"html"})"#,
            json!("cache/a1b2c3d4e5f6a1b2.html"),
        ),
        (
            r#"java.cacheFile("https://example.test/lib.js")"#,
            json!("cached script body\n"),
        ),
        (
            r#"java.importScript("https://example.test/h.js")"#,
            json!("function calc(){return 1+1;}\n"),
        ),
        (r#"java.setContent("<p>c</p>")"#, json!(null)),
        (r#"java.put("k","v")"#, json!("v")),
        (r#"java.reGetBook()"#, json!(null)),
    ];

    for (script, expected) in cases {
        let registry = observing_registry(Arc::clone(&calls));
        let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
        let result = sandbox
            .evaluate(script)
            .unwrap_or_else(|e| panic!("evaluating {script:?} failed: {e:?}"));
        assert_eq!(
            result.value, *expected,
            "response shape mismatch for {script:?}"
        );
    }

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), cases.len());
    // All descriptors carry their canonical java.* routing name.
    let names: Vec<&str> = calls.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        &[
            "java.downloadFile",
            "java.cacheFile",
            "java.importScript",
            "java.setContent",
            "java.put",
            "java.reGetBook",
        ]
    );
}

//! S3 network-class closure host-routing tests.
//!
//! Covers the 28 new `java.*` host-routed methods added in the S3 closure
//! round vs Legado `JsExtensions.kt` (79 methods total):
//!   - Network/HTTP: head
//!   - WebView/Browser: webView, webViewGetSource, webViewGetOverrideUrl,
//!     startBrowser, startBrowserAwait, getVerificationCode, getCookie
//!   - File/Archive: getFile, readFile, readTxtFile, deleteFile, unzipFile,
//!     un7zFile, unrarFile, unArchiveFile, getTxtInFolder, getZip/Rar/7z
//!     StringContent (×3), getZip/Rar/7zByteArrayContent (×3)
//!   - Font/TTF: queryBase64TTF, queryTTF, replaceFont
//!   - Device/UI: androidId, openUrl
//!
//! Each test verifies:
//!   1. The JS call routes through the `HostCallbackRegistry` (descriptor captured).
//!   2. The `HostDescriptor` variant carries the correct typed fields (mirror
//!      Legado signatures).
//!   3. The host's canned response round-trips back to JS.
//!   4. Arg validation fails closed on bad shapes (missing required, wrong type).

use reader_js::{
    HostCallbackRegistry, HostDescriptor, JsErrorKind, JsRuntimeConfig, JsSandbox, QuickJsSandbox,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

/// Registry that captures every `HostDescriptor` it receives and returns a
/// per-method canned response. Each test clones this and asserts on the sink.
fn observing_registry(sink: Arc<Mutex<Vec<HostDescriptor>>>) -> HostCallbackRegistry {
    let mut registry = HostCallbackRegistry::new();

    macro_rules! register {
        ($name:literal, $response:expr) => {{
            let s = Arc::clone(&sink);
            registry.register($name, move |descriptor| {
                s.lock().unwrap().push(descriptor.clone());
                Ok($response)
            });
        }};
    }

    register!("java.head", json!({"status": 200, "headers": {}}));
    register!("java.webView", json!("<html>webView body</html>"));
    register!(
        "java.webViewGetSource",
        json!("https://example.test/stream.m3u8")
    );
    register!(
        "java.webViewGetOverrideUrl",
        json!("https://example.test/redirect")
    );
    register!("java.startBrowser", json!(null));
    register!(
        "java.startBrowserAwait",
        json!({"url": "https://example.test", "body": "verified"})
    );
    register!("java.getVerificationCode", json!("ABCD"));
    register!("java.getCookie", json!("session=abc123"));
    register!(
        "java.getFile",
        json!({"path": "/cache/file.txt", "exists": true})
    );
    register!("java.readFile", json!("aGVsbG8=")); // base64
    register!("java.readTxtFile", json!("file contents\n"));
    register!("java.deleteFile", json!(true));
    register!("java.unzipFile", json!("cache/extracted/"));
    register!("java.un7zFile", json!("cache/extracted/"));
    register!("java.unrarFile", json!("cache/extracted/"));
    register!("java.unArchiveFile", json!("cache/extracted/"));
    register!("java.getTxtInFolder", json!("file1\nfile2"));
    register!("java.getZipStringContent", json!("chapter text"));
    register!("java.getRarStringContent", json!("chapter text"));
    register!("java.get7zStringContent", json!("chapter text"));
    register!("java.getZipByteArrayContent", json!("Ynl0ZXM="));
    register!("java.getRarByteArrayContent", json!("Ynl0ZXM="));
    register!("java.get7zByteArrayContent", json!("Ynl0ZXM="));
    register!(
        "java.queryBase64TTF",
        json!({"handle": "ttf-1", "glyphs": 256})
    );
    register!("java.queryTTF", json!({"handle": "ttf-1", "glyphs": 256}));
    register!("java.replaceFont", json!("replaced font text"));
    register!("java.androidId", json!("abcdef0123456789"));
    register!("java.openUrl", json!(null));

    registry
}

#[test]
fn routes_java_head_through_host_callback() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.head("https://example.test", { "Accept": "text/plain" })"#)
        .unwrap();
    // JS numbers come back as f64; compare numerically.
    assert_eq!(result.value["status"].as_f64(), Some(200.0));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::HttpHead { url, headers } => {
            assert_eq!(url, "https://example.test");
            assert!(headers.is_some());
        }
        other => panic!("expected HttpHead, got {other:?}"),
    }
}

#[test]
fn routes_java_webview_with_nullable_string_args() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.webView(null, "https://example.test", "document.title")"#)
        .unwrap();
    assert_eq!(result.value, json!("<html>webView body</html>"));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::WebView { html, url, js } => {
            assert!(html.is_none());
            assert_eq!(url.as_deref(), Some("https://example.test"));
            assert_eq!(js.as_deref(), Some("document.title"));
        }
        other => panic!("expected WebView, got {other:?}"),
    }
}

#[test]
fn routes_java_webview_get_source_with_regex() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.webViewGetSource(null, "https://example.test", null, "stream\\.m3u8")"#)
        .unwrap();
    assert_eq!(result.value, json!("https://example.test/stream.m3u8"));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::WebViewGetSource {
            html,
            url,
            js,
            source_regex,
        } => {
            assert!(html.is_none());
            assert_eq!(url.as_deref(), Some("https://example.test"));
            assert!(js.is_none());
            // JS string "stream\\.m3u8" evaluates to "stream\.m3u8" (one backslash).
            assert_eq!(source_regex, r"stream\.m3u8");
        }
        other => panic!("expected WebViewGetSource, got {other:?}"),
    }
}

#[test]
fn routes_java_webview_get_override_url() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.webViewGetOverrideUrl(null, "https://example.test", null, "redirect")"#)
        .unwrap();
    assert_eq!(result.value, json!("https://example.test/redirect"));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::WebViewGetOverrideUrl {
            override_url_regex, ..
        } => {
            assert_eq!(override_url_regex, "redirect");
        }
        other => panic!("expected WebViewGetOverrideUrl, got {other:?}"),
    }
}

#[test]
fn routes_java_start_browser() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.startBrowser("https://example.test", "Login")"#)
        .unwrap();
    assert_eq!(result.value, json!(null));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::StartBrowser { url, title } => {
            assert_eq!(url, "https://example.test");
            assert_eq!(title, "Login");
        }
        other => panic!("expected StartBrowser, got {other:?}"),
    }
}

#[test]
fn routes_java_start_browser_await_with_optional_bool() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    // 3-arg form: url, title, refetchAfterSuccess=true
    let result = sandbox
        .evaluate(r#"java.startBrowserAwait("https://example.test", "Captcha", true)"#)
        .unwrap();
    assert_eq!(result.value["body"], json!("verified"));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::StartBrowserAwait {
            url,
            title,
            refetch_after_success,
        } => {
            assert_eq!(url, "https://example.test");
            assert_eq!(title, "Captcha");
            assert_eq!(*refetch_after_success, Some(true));
        }
        other => panic!("expected StartBrowserAwait, got {other:?}"),
    }
}

#[test]
fn routes_java_start_browser_await_two_arg_form_defaults_refetch_to_none() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    // 2-arg form: url, title (legado defaults refetchAfterSuccess=true; reader-js
    // passes None and lets the host apply its own default).
    sandbox
        .evaluate(r#"java.startBrowserAwait("https://example.test", "Captcha")"#)
        .unwrap();

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::StartBrowserAwait {
            refetch_after_success,
            ..
        } => {
            assert_eq!(*refetch_after_success, None);
        }
        other => panic!("expected StartBrowserAwait, got {other:?}"),
    }
}

#[test]
fn routes_java_get_verification_code() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.getVerificationCode("https://example.test/captcha.png")"#)
        .unwrap();
    assert_eq!(result.value, json!("ABCD"));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::GetVerificationCode { image_url } => {
            assert_eq!(image_url, "https://example.test/captcha.png");
        }
        other => panic!("expected GetVerificationCode, got {other:?}"),
    }
}

#[test]
fn routes_java_get_cookie_with_optional_key() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    // tag-only form: returns full cookie header
    let result = sandbox
        .evaluate(r#"java.getCookie("example.test")"#)
        .unwrap();
    assert_eq!(result.value, json!("session=abc123"));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::GetCookie { tag, key } => {
            assert_eq!(tag, "example.test");
            assert!(key.is_none());
        }
        other => panic!("expected GetCookie, got {other:?}"),
    }
}

#[test]
fn routes_java_get_cookie_with_key_returns_specific_cookie() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    sandbox
        .evaluate(r#"java.getCookie("example.test", "session")"#)
        .unwrap();

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::GetCookie { tag, key } => {
            assert_eq!(tag, "example.test");
            assert_eq!(key.as_deref(), Some("session"));
        }
        other => panic!("expected GetCookie, got {other:?}"),
    }
}

#[test]
fn routes_java_file_methods_with_path_string() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    sandbox
        .evaluate(r#"java.getFile("cache/file.txt")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.readFile("cache/file.txt")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.deleteFile("cache/file.txt")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.unzipFile("cache/archive.zip")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.un7zFile("cache/archive.7z")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.unrarFile("cache/archive.rar")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.unArchiveFile("cache/archive")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.getTxtInFolder("cache/folder")"#)
        .unwrap();

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 8);
    // Spot-check a few variants
    match &calls[0] {
        HostDescriptor::GetFile { path } => assert_eq!(path, "cache/file.txt"),
        other => panic!("expected GetFile, got {other:?}"),
    }
    match &calls[1] {
        HostDescriptor::ReadFile { path } => assert_eq!(path, "cache/file.txt"),
        other => panic!("expected ReadFile, got {other:?}"),
    }
    match &calls[3] {
        HostDescriptor::UnzipFile { zip_path } => assert_eq!(zip_path, "cache/archive.zip"),
        other => panic!("expected UnzipFile, got {other:?}"),
    }
    match &calls[6] {
        HostDescriptor::UnArchiveFile { zip_path } => assert_eq!(zip_path, "cache/archive"),
        other => panic!("expected UnArchiveFile, got {other:?}"),
    }
}

#[test]
fn routes_java_read_txt_file_with_optional_charset() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    // 1-arg form: auto-detect charset
    sandbox
        .evaluate(r#"java.readTxtFile("cache/file.txt")"#)
        .unwrap();
    // 2-arg form: explicit charset
    sandbox
        .evaluate(r#"java.readTxtFile("cache/file.txt", "GBK")"#)
        .unwrap();

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::ReadTxtFile { path, charset } => {
            assert_eq!(path, "cache/file.txt");
            assert!(charset.is_none());
        }
        other => panic!("expected ReadTxtFile auto, got {other:?}"),
    }
    match &calls[1] {
        HostDescriptor::ReadTxtFile { path, charset } => {
            assert_eq!(path, "cache/file.txt");
            assert_eq!(charset.as_deref(), Some("GBK"));
        }
        other => panic!("expected ReadTxtFile GBK, got {other:?}"),
    }
}

#[test]
fn routes_java_archive_string_content_methods() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    sandbox
        .evaluate(r#"java.getZipStringContent("https://example.test/a.zip", "chapter1.txt")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.getRarStringContent("https://example.test/a.rar", "ch.txt", "GBK")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.get7zStringContent("https://example.test/a.7z", "ch.txt")"#)
        .unwrap();

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::GetZipStringContent { url, path, charset } => {
            assert_eq!(url, "https://example.test/a.zip");
            assert_eq!(path, "chapter1.txt");
            assert!(charset.is_none());
        }
        other => panic!("expected GetZipStringContent, got {other:?}"),
    }
    match &calls[1] {
        HostDescriptor::GetRarStringContent { url, path, charset } => {
            assert_eq!(url, "https://example.test/a.rar");
            assert_eq!(path, "ch.txt");
            assert_eq!(charset.as_deref(), Some("GBK"));
        }
        other => panic!("expected GetRarStringContent, got {other:?}"),
    }
}

#[test]
fn routes_java_archive_byte_array_content_methods() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    sandbox
        .evaluate(r#"java.getZipByteArrayContent("https://example.test/a.zip", "image.png")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.getRarByteArrayContent("https://example.test/a.rar", "image.png")"#)
        .unwrap();
    sandbox
        .evaluate(r#"java.get7zByteArrayContent("https://example.test/a.7z", "image.png")"#)
        .unwrap();

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    match &calls[0] {
        HostDescriptor::GetZipByteArrayContent { url, path } => {
            assert_eq!(url, "https://example.test/a.zip");
            assert_eq!(path, "image.png");
        }
        other => panic!("expected GetZipByteArrayContent, got {other:?}"),
    }
}

#[test]
fn routes_java_query_ttf_with_optional_use_cache() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    // 1-arg form: useCache defaults to true in legado; reader-js passes None
    sandbox
        .evaluate(r#"java.queryTTF("https://example.test/font.ttf")"#)
        .unwrap();
    // 2-arg form: explicit useCache=false
    sandbox
        .evaluate(r#"java.queryTTF("https://example.test/font.ttf", false)"#)
        .unwrap();

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::QueryTTF { data, use_cache } => {
            assert_eq!(data, &json!("https://example.test/font.ttf"));
            assert_eq!(*use_cache, None);
        }
        other => panic!("expected QueryTTF, got {other:?}"),
    }
    match &calls[1] {
        HostDescriptor::QueryTTF { use_cache, .. } => {
            assert_eq!(*use_cache, Some(false));
        }
        other => panic!("expected QueryTTF false, got {other:?}"),
    }
}

#[test]
fn routes_java_query_base64_ttf_as_deprecated_alias() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(r#"java.queryBase64TTF("AAEAAAA...")"#)
        .unwrap();
    assert_eq!(result.value["handle"], json!("ttf-1"));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::QueryBase64TTF { data } => {
            assert_eq!(data, "AAEAAAA...");
        }
        other => panic!("expected QueryBase64TTF, got {other:?}"),
    }
}

#[test]
fn routes_java_replace_font_with_optional_filter() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    // 3-arg form: filter defaults to false in legado; reader-js passes None
    sandbox
        .evaluate(r#"java.replaceFont("obfuscated", {"handle":"err"}, {"handle":"ok"})"#)
        .unwrap();
    // 4-arg form: explicit filter=true
    sandbox
        .evaluate(r#"java.replaceFont("obfuscated", {"handle":"err"}, {"handle":"ok"}, true)"#)
        .unwrap();

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::ReplaceFont {
            text,
            error_query_ttf,
            correct_query_ttf,
            filter,
        } => {
            assert_eq!(text, "obfuscated");
            assert_eq!(error_query_ttf, &json!({"handle":"err"}));
            assert_eq!(correct_query_ttf, &json!({"handle":"ok"}));
            assert_eq!(*filter, None);
        }
        other => panic!("expected ReplaceFont, got {other:?}"),
    }
    match &calls[1] {
        HostDescriptor::ReplaceFont { filter, .. } => {
            assert_eq!(*filter, Some(true));
        }
        other => panic!("expected ReplaceFont filter=true, got {other:?}"),
    }
}

#[test]
fn routes_java_android_id_with_no_args() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox.evaluate(r#"java.androidId()"#).unwrap();
    assert_eq!(result.value, json!("abcdef0123456789"));

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::AndroidId => {}
        other => panic!("expected AndroidId, got {other:?}"),
    }
}

#[test]
fn routes_java_open_url_with_optional_mime_type() {
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    // 1-arg form
    sandbox
        .evaluate(r#"java.openUrl("https://example.test")"#)
        .unwrap();
    // 2-arg form with mimeType
    sandbox
        .evaluate(r#"java.openUrl("https://example.test", "text/html")"#)
        .unwrap();

    let calls = calls.lock().unwrap();
    match &calls[0] {
        HostDescriptor::OpenUrl { url, mime_type } => {
            assert_eq!(url, "https://example.test");
            assert!(mime_type.is_none());
        }
        other => panic!("expected OpenUrl, got {other:?}"),
    }
    match &calls[1] {
        HostDescriptor::OpenUrl { url, mime_type } => {
            assert_eq!(url, "https://example.test");
            assert_eq!(mime_type.as_deref(), Some("text/html"));
        }
        other => panic!("expected OpenUrl mime, got {other:?}"),
    }
}

// ===== Fail-closed validation tests =====
// Arg validation errors (Exception::throw_type) → JsErrorKind::Exception.
// Unregistered callback errors (Exception::throw_internal from registry.call)
// → JsErrorKind::HostCallback.

#[test]
fn head_without_url_throws_exception() {
    let registry = HostCallbackRegistry::new();
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
    let error = sandbox.evaluate(r#"java.head()"#).unwrap_err();
    assert_eq!(error.kind, JsErrorKind::Exception);
    assert!(error.message.contains("java.head"));
}

#[test]
fn webview_get_source_without_regex_throws_exception() {
    let registry = HostCallbackRegistry::new();
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
    let error = sandbox
        .evaluate(r#"java.webViewGetSource(null, "https://example.test", null)"#)
        .unwrap_err();
    assert_eq!(error.kind, JsErrorKind::Exception);
}

#[test]
fn start_browser_with_one_arg_throws_exception() {
    let registry = HostCallbackRegistry::new();
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
    let error = sandbox
        .evaluate(r#"java.startBrowser("https://example.test")"#)
        .unwrap_err();
    assert_eq!(error.kind, JsErrorKind::Exception);
}

#[test]
fn get_zip_string_content_with_one_arg_throws_exception() {
    let registry = HostCallbackRegistry::new();
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
    let error = sandbox
        .evaluate(r#"java.getZipStringContent("https://example.test/a.zip")"#)
        .unwrap_err();
    assert_eq!(error.kind, JsErrorKind::Exception);
}

#[test]
fn query_ttf_without_data_throws_exception() {
    let registry = HostCallbackRegistry::new();
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
    let error = sandbox.evaluate(r#"java.queryTTF()"#).unwrap_err();
    assert_eq!(error.kind, JsErrorKind::Exception);
}

#[test]
fn replace_font_with_two_args_throws_exception() {
    let registry = HostCallbackRegistry::new();
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
    let error = sandbox
        .evaluate(r#"java.replaceFont("text", {"handle":"err"})"#)
        .unwrap_err();
    assert_eq!(error.kind, JsErrorKind::Exception);
}

#[test]
fn unregistered_callback_fails_closed_for_new_methods() {
    // No callbacks registered — every S3 method with valid args should fail
    // closed with JsErrorKind::HostCallback (registry.call returns Err, which
    // throws_internal → mapped to HostCallback).
    let registry = HostCallbackRegistry::new();
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let error = sandbox
        .evaluate(r#"java.head("https://example.test")"#)
        .unwrap_err();
    assert_eq!(error.kind, JsErrorKind::HostCallback);
    assert!(error.message.contains("java.head"));

    let error = sandbox.evaluate(r#"java.androidId()"#).unwrap_err();
    assert_eq!(error.kind, JsErrorKind::HostCallback);
    assert!(error.message.contains("java.androidId"));
}

#[test]
fn full_s3_surface_round_trips_through_host_callback() {
    // End-to-end: a realistic Legado-style JS snippet that exercises multiple
    // S3 methods in sequence. Mirrors the kind of <js>…</js> rule block found
    // in real book sources (webview-based login + cookie extraction + archive
    // chapter fetch + font de-obfuscation).
    let calls = Arc::new(Mutex::new(Vec::<HostDescriptor>::new()));
    let registry = observing_registry(Arc::clone(&calls));
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let result = sandbox
        .evaluate(
            r#"
            // 1. WebView-based source fetch
            var html = java.webView(null, "https://example.test/ch1", "document.body.innerText");
            // 2. Cookie extraction for the next request
            var cookie = java.getCookie("example.test", "session");
            // 3. Read a chapter from a zip archive
            var chapter = java.getZipStringContent("https://example.test/book.zip", "ch1.txt", "UTF-8");
            // 4. De-obfuscate font
            var ttf = java.queryTTF("https://example.test/font.ttf");
            var clean = java.replaceFont(chapter, ttf, ttf, true);
            clean
            "#,
        )
        .unwrap();

    assert_eq!(result.value, json!("replaced font text"));

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 5);
    // Verify the descriptor order matches the call sequence
    assert!(matches!(calls[0], HostDescriptor::WebView { .. }));
    assert!(matches!(calls[1], HostDescriptor::GetCookie { .. }));
    assert!(matches!(
        calls[2],
        HostDescriptor::GetZipStringContent { .. }
    ));
    assert!(matches!(calls[3], HostDescriptor::QueryTTF { .. }));
    assert!(matches!(calls[4], HostDescriptor::ReplaceFont { .. }));
}

//! P3 follow-up: strong-typed HostDescriptor boundary.
//!
//! These tests assert that host callbacks receive a strong-typed
//! `HostDescriptor` variant (with parsed, typed fields) instead of the old
//! weak `HostCall { name, args: Vec<JsonValue> }` shape. The host no longer
//! has to switch on `call.name` and re-parse `call.args` — it pattern-matches
//! on the variant directly.
//!
//! This is the migration target described in the user directive: "新建
//! HostDescriptor 类型（带 method/kind/args 强类型字段），迁移 ajax/get/post
//! + residual 全部 host 路由到新类型".

use reader_js::{HostCallbackRegistry, HostDescriptor, JsRuntimeConfig, JsSandbox, QuickJsSandbox};
use serde_json::json;
use std::sync::{Arc, Mutex};

/// Helper: observe the descriptor a callback receives, return it for assertion.
fn capture_descriptor(script: &str, register_name: &str) -> reader_js::HostDescriptor {
    let captured: Arc<Mutex<Option<HostDescriptor>>> = Arc::new(Mutex::new(None));
    let mut registry = HostCallbackRegistry::new();
    let sink = Arc::clone(&captured);
    registry.register(register_name, move |descriptor| {
        *sink.lock().unwrap() = Some(descriptor);
        Ok(json!("stub"))
    });
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
    sandbox.evaluate(script).unwrap();
    let captured = captured.lock().unwrap().take();
    captured.expect("host callback was not invoked")
}

#[test]
fn java_get_routes_as_http_get_descriptor_with_url_and_headers() {
    let descriptor = capture_descriptor(
        r#"java.get("https://example.test/chapter", {"Referer": "https://example.test"})"#,
        "java.get",
    );
    match descriptor {
        HostDescriptor::HttpGet { url, headers } => {
            assert_eq!(url, "https://example.test/chapter");
            assert_eq!(headers, Some(json!({"Referer": "https://example.test"})));
        }
        other => panic!("expected HttpGet, got {other:?}"),
    }
}

#[test]
fn java_get_with_only_url_routes_as_http_get_with_none_headers() {
    let descriptor = capture_descriptor(r#"java.get("https://example.test/only-url")"#, "java.get");
    match descriptor {
        HostDescriptor::HttpGet { url, headers } => {
            assert_eq!(url, "https://example.test/only-url");
            assert_eq!(headers, None);
        }
        other => panic!("expected HttpGet, got {other:?}"),
    }
}

#[test]
fn java_post_routes_as_http_post_descriptor_with_body_and_headers() {
    let descriptor = capture_descriptor(
        r#"java.post("https://example.test/submit", "body-text", {"X-Trace": "abc"})"#,
        "java.post",
    );
    match descriptor {
        HostDescriptor::HttpPost { url, body, headers } => {
            assert_eq!(url, "https://example.test/submit");
            assert_eq!(body, "body-text");
            assert_eq!(headers, Some(json!({"X-Trace": "abc"})));
        }
        other => panic!("expected HttpPost, got {other:?}"),
    }
}

#[test]
fn java_post_missing_body_still_routes_with_empty_body_string() {
    // legado leniency: missing body arg → empty body string, not None.
    let descriptor =
        capture_descriptor(r#"java.post("https://example.test/no-body")"#, "java.post");
    match descriptor {
        HostDescriptor::HttpPost { url, body, headers } => {
            assert_eq!(url, "https://example.test/no-body");
            assert_eq!(body, "");
            assert_eq!(headers, None);
        }
        other => panic!("expected HttpPost, got {other:?}"),
    }
}

#[test]
fn java_ajax_routes_as_ajax_descriptor_with_url_only() {
    // legado's ajax returns String? body — no headers/code exposed. The
    // descriptor carries just the url; the host returns the body.
    let descriptor = capture_descriptor(r#"java.ajax("https://example.test/ajax")"#, "java.ajax");
    match descriptor {
        HostDescriptor::Ajax { url } => {
            assert_eq!(url, "https://example.test/ajax");
        }
        other => panic!("expected Ajax, got {other:?}"),
    }
}

#[test]
fn java_ajax_all_routes_as_ajax_all_with_url_vector() {
    let descriptor = capture_descriptor(
        r#"java.ajaxAll(["https://example.test/a", "https://example.test/b"])"#,
        "java.ajaxAll",
    );
    match descriptor {
        HostDescriptor::AjaxAll { urls } => {
            assert_eq!(
                urls,
                vec!["https://example.test/a", "https://example.test/b"]
            );
        }
        other => panic!("expected AjaxAll, got {other:?}"),
    }
}

#[test]
fn java_get_source_routes_as_unit_variant_get_source() {
    let descriptor = capture_descriptor(r#"java.getSource()"#, "java.getSource");
    assert_eq!(descriptor, HostDescriptor::GetSource);
}

#[test]
fn java_get_string_routes_as_get_string_with_rule_field() {
    let descriptor = capture_descriptor(r#"java.getString("$.title")"#, "java.getString");
    match descriptor {
        HostDescriptor::GetString { rule } => {
            assert_eq!(rule, "$.title");
        }
        other => panic!("expected GetString, got {other:?}"),
    }
}

#[test]
fn java_get_string_list_routes_as_get_string_list_with_rule_field() {
    let descriptor = capture_descriptor(
        r#"java.getStringList("$.items[*].name")"#,
        "java.getStringList",
    );
    match descriptor {
        HostDescriptor::GetStringList { rule } => {
            assert_eq!(rule, "$.items[*].name");
        }
        other => panic!("expected GetStringList, got {other:?}"),
    }
}

#[test]
fn java_download_file_routes_as_download_file_descriptor_with_url() {
    let descriptor = capture_descriptor(
        r#"java.downloadFile("https://example.test/img.png")"#,
        "java.downloadFile",
    );
    match descriptor {
        HostDescriptor::DownloadFile { url } => {
            assert_eq!(url, "https://example.test/img.png");
        }
        other => panic!("expected DownloadFile, got {other:?}"),
    }
}

#[test]
fn java_cache_file_routes_as_cache_file_with_optional_save_time() {
    let descriptor = capture_descriptor(
        r#"java.cacheFile("https://example.test/lib.js", 3600)"#,
        "java.cacheFile",
    );
    match descriptor {
        HostDescriptor::CacheFile { url, save_time } => {
            assert_eq!(url, "https://example.test/lib.js");
            assert_eq!(save_time, Some(3600));
        }
        other => panic!("expected CacheFile, got {other:?}"),
    }
}

#[test]
fn java_cache_file_without_save_time_routes_with_none_save_time() {
    let descriptor = capture_descriptor(
        r#"java.cacheFile("https://example.test/lib.js")"#,
        "java.cacheFile",
    );
    match descriptor {
        HostDescriptor::CacheFile { url, save_time } => {
            assert_eq!(url, "https://example.test/lib.js");
            assert_eq!(save_time, None);
        }
        other => panic!("expected CacheFile, got {other:?}"),
    }
}

#[test]
fn java_import_script_routes_as_import_script_with_path() {
    let descriptor = capture_descriptor(
        r#"java.importScript("https://example.test/h.js")"#,
        "java.importScript",
    );
    match descriptor {
        HostDescriptor::ImportScript { path } => {
            assert_eq!(path, "https://example.test/h.js");
        }
        other => panic!("expected ImportScript, got {other:?}"),
    }
}

#[test]
fn java_set_content_routes_as_set_content_with_content_and_base_url() {
    let descriptor = capture_descriptor(
        r#"java.setContent("<p>c</p>", "https://example.test/base")"#,
        "java.setContent",
    );
    match descriptor {
        HostDescriptor::SetContent { content, base_url } => {
            assert_eq!(content, Some("<p>c</p>".to_string()));
            assert_eq!(base_url, Some("https://example.test/base".to_string()));
        }
        other => panic!("expected SetContent, got {other:?}"),
    }
}

#[test]
fn java_set_content_with_only_content_routes_with_none_base_url() {
    let descriptor = capture_descriptor(r#"java.setContent("<p>c</p>")"#, "java.setContent");
    match descriptor {
        HostDescriptor::SetContent { content, base_url } => {
            assert_eq!(content, Some("<p>c</p>".to_string()));
            assert_eq!(base_url, None);
        }
        other => panic!("expected SetContent, got {other:?}"),
    }
}

#[test]
fn java_put_routes_as_put_descriptor_with_key_and_value() {
    let descriptor = capture_descriptor(r#"java.put("varName", "varValue")"#, "java.put");
    match descriptor {
        HostDescriptor::Put { key, value } => {
            assert_eq!(key, "varName");
            assert_eq!(value, "varValue");
        }
        other => panic!("expected Put, got {other:?}"),
    }
}

#[test]
fn java_re_get_book_routes_as_unit_variant_re_get_book() {
    let descriptor = capture_descriptor(r#"java.reGetBook()"#, "java.reGetBook");
    assert_eq!(descriptor, HostDescriptor::ReGetBook);
}

#[test]
fn java_connect_routes_as_http_connect_with_url_and_optional_header() {
    let descriptor = capture_descriptor(
        r#"java.connect("https://example.test/conn", "{\"Referer\":\"https://example.test\"}")"#,
        "java.connect",
    );
    match descriptor {
        HostDescriptor::HttpConnect { url, header } => {
            assert_eq!(url, "https://example.test/conn");
            assert_eq!(
                header,
                Some("{\"Referer\":\"https://example.test\"}".to_string())
            );
        }
        other => panic!("expected HttpConnect, got {other:?}"),
    }
}

#[test]
fn descriptor_carries_strong_typing_so_host_pattern_matches_without_name_switch() {
    // The whole point of HostDescriptor: the host pattern-matches on the
    // variant, never switches on a string name. This test exercises a single
    // callback registered for two different method names and proves the host
    // can distinguish them by variant, not by string.
    let observed: Arc<Mutex<Vec<HostDescriptor>>> = Arc::new(Mutex::new(Vec::new()));
    let mut registry = HostCallbackRegistry::new();
    let sink = Arc::clone(&observed);
    registry.register("java.get", move |d| {
        sink.lock().unwrap().push(d);
        Ok(json!("get-stub"))
    });
    let sink2 = Arc::clone(&observed);
    registry.register("java.put", move |d| {
        sink2.lock().unwrap().push(d);
        Ok(json!("put-stub"))
    });
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);
    sandbox
        .evaluate(r#"java.get("https://example.test/a"); java.put("k","v")"#)
        .unwrap();

    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert!(matches!(observed[0], HostDescriptor::HttpGet { .. }));
    assert!(matches!(observed[1], HostDescriptor::Put { .. }));
}

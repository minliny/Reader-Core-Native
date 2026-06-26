use reader_js::{
    CancellationToken, HostCallbackRegistry, HostDescriptor, JsErrorKind, JsExecutionOptions,
    JsRuntimeConfig, JsSandbox, QuickJsSandbox,
};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[test]
fn chinese_conversion_helpers_match_old_core_postprocessor_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                globalT2s: t2s("國家圖書館學會"),
                javaT2s: java.t2s("國家圖書館學會"),
                globalS2t: s2t("中国图书馆"),
                javaS2t: java.s2t("中国图书馆")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "globalT2s": "国家图书馆学会",
            "javaT2s": "国家图书馆学会",
            "globalS2t": "中國圖書館",
            "javaS2t": "中國圖書館"
        })
    );
}

#[test]
fn timeout_and_cancel_boundaries_are_enforced() {
    let timeout_sandbox = QuickJsSandbox::new(JsRuntimeConfig {
        timeout: Some(Duration::from_millis(10)),
        ..JsRuntimeConfig::default()
    });

    let timeout = timeout_sandbox.evaluate("while (true) {}").unwrap_err();
    assert_eq!(timeout.kind, JsErrorKind::Timeout);

    let token = CancellationToken::new();
    token.cancel();
    let cancelled = QuickJsSandbox::default()
        .evaluate_with_options(
            "1 + 1",
            JsExecutionOptions {
                cancellation_token: Some(token),
                ..JsExecutionOptions::default()
            },
        )
        .unwrap_err();
    assert_eq!(cancelled.kind, JsErrorKind::Cancelled);
}

#[test]
fn host_callback_stub_routes_without_implementing_network() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&calls);
    let mut registry = HostCallbackRegistry::new();
    registry.register("java.ajax", move |call| {
        observed.lock().unwrap().push(call);
        Ok(json!({"body": "<p>stubbed</p>", "source": "host-stub"}))
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
        json!({"body": "<p>stubbed</p>", "source": "host-stub"})
    );

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
fn host_callback_stub_observes_execution_timeout_after_returning() {
    let mut registry = HostCallbackRegistry::new();
    registry.register("java.ajax", |_| {
        thread::sleep(Duration::from_millis(25));
        Ok(json!("late host result"))
    });
    let sandbox = QuickJsSandbox::with_host_callbacks(
        JsRuntimeConfig {
            timeout: Some(Duration::from_millis(1)),
            ..JsRuntimeConfig::default()
        },
        registry,
    );

    let error = sandbox
        .evaluate(r#"java.ajax("https://example.test/slow")"#)
        .unwrap_err();

    assert_eq!(error.kind, JsErrorKind::Timeout);
}

#[test]
fn host_callback_stub_observes_cancellation_after_returning() {
    let token = CancellationToken::new();
    let callback_token = token.clone();
    let mut registry = HostCallbackRegistry::new();
    registry.register("java.ajax", move |_| {
        callback_token.cancel();
        Ok(json!("cancelled host result"))
    });
    let sandbox = QuickJsSandbox::with_host_callbacks(JsRuntimeConfig::default(), registry);

    let error = sandbox
        .evaluate_with_options(
            r#"java.ajax("https://example.test/cancel")"#,
            JsExecutionOptions {
                cancellation_token: Some(token),
                ..JsExecutionOptions::default()
            },
        )
        .unwrap_err();

    assert_eq!(error.kind, JsErrorKind::Cancelled);
}

#[test]
fn webview_browser_and_captcha_apis_remain_host_bound() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                webView: typeof java.webView,
                webViewGetSource: typeof java.webViewGetSource,
                startBrowser: typeof java.startBrowser,
                startBrowserAwait: typeof java.startBrowserAwait,
                getVerificationCode: typeof java.getVerificationCode,
                javaOpenUrl: typeof java.openUrl,
                globalOpenUrl: typeof openUrl
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "webView": "undefined",
            "webViewGetSource": "undefined",
            "startBrowser": "undefined",
            "startBrowserAwait": "undefined",
            "getVerificationCode": "undefined",
            "javaOpenUrl": "undefined",
            "globalOpenUrl": "undefined"
        })
    );
}

#[test]
fn gbk_charset_helpers_cover_common_search_keyword_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                encodedUri: java.encodeURI("鬼吹灯", "GBK"),
                encodedComponent: encodeURIComponent("鬼吹灯", "GBK"),
                bytes: java.strToBytes("鬼吹灯", "GBK"),
                text: java.bytesToStr("b9edb4b5b5c6", "GBK"),
                decodedBase64: java.base64Decode("ue20tbXG", "GBK"),
                submitEncoded: java.encodeURI("搜索", "GBK"),
                submitBytes: java.strToBytes("搜索", "GBK"),
                submitText: java.bytesToStr("cbd1cbf7", "GBK"),
                submitBase64: java.base64Decode("y9HL9w==", "GBK"),
                gbkSubmitEncoded: java.encodeURI("提交", "GBK"),
                gbkSubmitBytes: java.strToBytes("提交", "GBK"),
                gbkSubmitText: java.bytesToStr("cce1bdbb", "GBK"),
                gbkSubmitBase64: java.base64Decode("zOG9uw==", "GBK")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "encodedUri": "%B9%ED%B4%B5%B5%C6",
            "encodedComponent": "%B9%ED%B4%B5%B5%C6",
            "bytes": "b9edb4b5b5c6",
            "text": "鬼吹灯",
            "decodedBase64": "鬼吹灯",
            "submitEncoded": "%CB%D1%CB%F7",
            "submitBytes": "cbd1cbf7",
            "submitText": "搜索",
            "submitBase64": "搜索",
            "gbkSubmitEncoded": "%CC%E1%BD%BB",
            "gbkSubmitBytes": "cce1bdbb",
            "gbkSubmitText": "提交",
            "gbkSubmitBase64": "提交"
        })
    );
}

#[test]
fn base64_decoder_alias_matches_legacy_guarded_corpus_probe() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                javaAlias: java.base64Decoder("cmVhZGVy"),
                globalAlias: base64Decoder("cmVhZGVy"),
                javaCharsetAlias: java.base64Decoder("0KHLtQ==", "GBK")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "javaAlias": "reader",
            "globalAlias": "reader",
            "javaCharsetAlias": "小说"
        })
    );
}

#[test]
fn legacy_escape_unescape_helpers_cover_guarded_corpus_patterns() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                escaped: escape("操嫩 a@*_+-./"),
                unescaped: unescape("%u64CD%u5AE9%20%3A"),
                replacedUnicodeEscape: unescape("\\u64cd".replace(/\\u/g, "%u"))
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "escaped": "%u64CD%u5AE9%20a@*_+-./",
            "unescaped": "操嫩 :",
            "replacedUnicodeEscape": "操"
        })
    );
}

#[test]
fn create_sign_canonicalizes_sorted_params_like_old_core_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                javaSign: java.createSign({
                    timestamp: 12345,
                    spaceId: "mp-a23",
                    empty: "",
                    method: "serverless.auth.user.anonymousAuthorize"
                }, "secret", "HMAC-MD5"),
                globalSign: createSign({
                    timestamp: 12345,
                    spaceId: "mp-a23",
                    empty: "",
                    method: "serverless.auth.user.anonymousAuthorize"
                }, "secret")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "javaSign": "3e6efed9198e9fc405c27b8281469914",
            "globalSign": "3e6efed9198e9fc405c27b8281469914"
        })
    );
}

#[test]
fn sha1_digest_aliases_match_old_core_digest_hex_api() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                globalSha1: hashDigest("abc", "SHA-1"),
                javaSha1: java.hashDigest("abc", "SHA1"),
                javaDigestHex: java.digestHex("abc", "SHA-1"),
                javaDigestBase64: java.digestBase64Str("abc", "SHA-1")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "globalSha1": "a9993e364706816aba3e25717850c26c9cd0d89d",
            "javaSha1": "a9993e364706816aba3e25717850c26c9cd0d89d",
            "javaDigestHex": "a9993e364706816aba3e25717850c26c9cd0d89d",
            "javaDigestBase64": "qZk+NkcGgWq6PiVxeFDCbJzQ2J0="
        })
    );
}

#[test]
fn sha512_digest_aliases_match_old_core_digest_hex_api() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                globalSha512: hashDigest("abc", "SHA-512"),
                javaSha512: java.hashDigest("abc", "SHA512"),
                javaDigestHex: java.digestHex("abc", "SHA-512"),
                javaDigestBase64: java.digestBase64Str("abc", "SHA-512")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "globalSha512": "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            "javaSha512": "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            "javaDigestHex": "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            "javaDigestBase64": "3a81oZNherrMQXNJriBBMRLm+k6JqX6iCp7u5ktV05ohkpkqJ0/BqDa6PCOj/uu9RU1EI2Q86A4qmslPpUyknw=="
        })
    );
}

#[test]
fn sm3_digest_aliases_match_old_core_hash_digest_api() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                globalSm3: hashDigest("abc", "SM3"),
                javaSm3: java.hashDigest("abc", "sm3"),
                javaDigestHex: java.digestHex("abc", "SM3"),
                javaDigestBase64: java.digestBase64Str("abc", "SM3"),
                normalizedEmpty: java.digestHex("", " sm3 ")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "globalSm3": "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0",
            "javaSm3": "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0",
            "javaDigestHex": "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0",
            "javaDigestBase64": "Zsfw9GLu7dnR8tRr3BDk4kFnxIdc8veiKX2gK49LqOA=",
            "normalizedEmpty": "1ab21d8355cfa17f8e61194831e81a8f22bec8c728fefb747ed035eb5082aa2b"
        })
    );
}

#[test]
fn unsupported_digest_algorithm_fails_closed_like_old_core_digest_hex() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                globalHashDigest: hashDigest("abc", "BLAKE2"),
                javaHashDigest: java.hashDigest("abc", "BLAKE2"),
                javaDigestHex: java.digestHex("abc", "BLAKE2"),
                javaDigestBase64: java.digestBase64Str("abc", "BLAKE2")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "globalHashDigest": "",
            "javaHashDigest": "",
            "javaDigestHex": "",
            "javaDigestBase64": ""
        })
    );
}

#[test]
fn to_url_query_only_relative_keeps_base_file_path_like_old_core() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var resolved = toURL("?page=2#frag", "https://owned.example/root/index.html?old=1");
            var javaResolved = java.toURL("?page=2#frag", "https://owned.example/root/index.html?old=1");
            ({
                text: String(resolved),
                javaText: String(javaResolved),
                pathname: resolved.pathname,
                page: resolved.searchParams.page
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "text": "https://owned.example/root/index.html?page=2#frag",
            "javaText": "https://owned.example/root/index.html?page=2#frag",
            "pathname": "/root/index.html",
            "page": "2"
        })
    );
}

#[test]
fn to_url_trims_absolute_input_like_old_core() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var resolved = toURL("  https://owned.example/root/index.html?x=1  ");
            var javaResolved = java.toURL("\nhttps://owned.example/root/index.html?x=1\t");
            ({
                text: String(resolved),
                javaText: String(javaResolved),
                host: resolved.host,
                pathname: resolved.pathname,
                x: resolved.searchParams.x
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "text": "https://owned.example/root/index.html?x=1",
            "javaText": "https://owned.example/root/index.html?x=1",
            "host": "owned.example",
            "pathname": "/root/index.html",
            "x": "1"
        })
    );
}

#[test]
fn to_url_blank_input_returns_empty_string_like_old_core() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                globalBlank: toURL("   \n\t  "),
                javaBlank: java.toURL("")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "globalBlank": "",
            "javaBlank": ""
        })
    );
}

#[test]
fn to_url_relative_input_without_base_returns_trimmed_input_like_old_core() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                relative: String(toURL("  ../chapter.html?x=1  ")),
                javaRelative: String(java.toURL("/book/1"))
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "relative": "../chapter.html?x=1",
            "javaRelative": "/book/1"
        })
    );
}

#[test]
fn to_url_invalid_base_returns_trimmed_input_like_old_core() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                relative: String(toURL("  ../chapter.html?x=1  ", "not a url")),
                javaRelative: String(java.toURL("/book/1", "bad base"))
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "relative": "../chapter.html?x=1",
            "javaRelative": "/book/1"
        })
    );
}

#[test]
fn to_url_fragment_only_relative_preserves_base_path_and_query_like_old_core() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r##"
            var resolved = toURL("#frag", "https://owned.example/root/index.html?old=1");
            var javaResolved = java.toURL("#frag", "https://owned.example/root/index.html?old=1");
            ({
                text: String(resolved),
                javaText: String(javaResolved),
                pathname: resolved.pathname,
                old: resolved.searchParams.old
            })
            "##,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "text": "https://owned.example/root/index.html?old=1#frag",
            "javaText": "https://owned.example/root/index.html?old=1#frag",
            "pathname": "/root/index.html",
            "old": "1"
        })
    );
}

#[test]
fn hmac_sha1_base64_matches_old_source_qidian_key_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var aid = "device123";
            var cid = "chapter456";
            var sha1 = "0" + aid + cid + "2EEE1433A152E84B3756301D8FA3E69A";
            ({
                javaBase64: java.HMacBase64(sha1, "HMAC-SHA1", aid),
                globalBase64: HMacBase64(sha1, "HMAC-SHA1", aid),
                javaHex: java.HMacHex(sha1, "HMAC-SHA1", aid),
                key0: java.HMacBase64(sha1, "HMAC-SHA1", aid).slice(0, -4)
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "javaBase64": "jw0sA7L3vg0LZ+4HQ0qug0Rxqug=",
            "globalBase64": "jw0sA7L3vg0LZ+4HQ0qug0Rxqug=",
            "javaHex": "8f0d2c03b2f7be0d0b67ee07434aae834471aae8",
            "key0": "jw0sA7L3vg0LZ+4HQ0qug0Rx"
        })
    );
}

#[test]
fn hmac_sha512_aliases_match_old_core_hmac_helper() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                globalDigest: hmacDigest("abc", "HMAC-SHA512", "key"),
                javaHex: java.HMacHex("abc", "HMAC-SHA-512", "key"),
                globalHex: HMacHex("abc", "HMAC-SHA512", "key"),
                javaBase64: java.HMacBase64("abc", "HMAC-SHA-512", "key"),
                globalBase64: HMacBase64("abc", "HMAC-SHA512", "key")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "globalDigest": "3926a207c8c42b0c41792cbd3e1a1aaaf5f7a25704f62dfc939c4987dd7ce060009c5bb1c2447355b3216f10b537e9afa7b64a4e5391b0d631172d07939e087a",
            "javaHex": "3926a207c8c42b0c41792cbd3e1a1aaaf5f7a25704f62dfc939c4987dd7ce060009c5bb1c2447355b3216f10b537e9afa7b64a4e5391b0d631172d07939e087a",
            "globalHex": "3926a207c8c42b0c41792cbd3e1a1aaaf5f7a25704f62dfc939c4987dd7ce060009c5bb1c2447355b3216f10b537e9afa7b64a4e5391b0d631172d07939e087a",
            "javaBase64": "OSaiB8jEKwxBeSy9PhoaqvX3olcE9i38k5xJh9184GAAnFuxwkRzVbMhbxC1N+mvp7ZKTlORsNYxFy0Hk54Ieg==",
            "globalBase64": "OSaiB8jEKwxBeSy9PhoaqvX3olcE9i38k5xJh9184GAAnFuxwkRzVbMhbxC1N+mvp7ZKTlORsNYxFy0Hk54Ieg=="
        })
    );
}

#[test]
fn aes_cbc_pkcs7_helpers_match_old_core_symmetric_crypto_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var cipher = java.createSymmetricCrypto(
                "AES/CBC/PKCS7Padding",
                "1234567890123456",
                "abcdefghijklmnop"
            );
            ({
                encrypted: cipher.encrypt("hello reader core"),
                decrypted: cipher.decryptStr("TV91HgttAaqcCuIa87buLCtI7lGk9I+P7cXIotvFstA="),
                globalDecrypted: createSymmetricCrypto(
                    "AES/CBC/PKCS5Padding",
                    "1234567890123456",
                    "abcdefghijklmnop"
                ).decryptStr("TV91HgttAaqcCuIa87buLCtI7lGk9I+P7cXIotvFstA="),
                legacyAesDecode: java.aesBase64DecodeToString(
                    "TV91HgttAaqcCuIa87buLCtI7lGk9I+P7cXIotvFstA=",
                    "1234567890123456",
                    "AES/CBC/PKCS5Padding",
                    "abcdefghijklmnop"
                )
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "encrypted": "TV91HgttAaqcCuIa87buLCtI7lGk9I+P7cXIotvFstA=",
            "decrypted": "hello reader core",
            "globalDecrypted": "hello reader core",
            "legacyAesDecode": "hello reader core"
        })
    );
}

#[test]
fn des_cbc_pkcs5_helper_matches_old_core_symmetric_crypto_roundtrip() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var cipher = java.createSymmetricCrypto(
                "DES/CBC/PKCS5Padding",
                "12345678",
                "abcdefgh"
            );
            ({
                encrypted: cipher.encrypt("hello des"),
                decrypted: cipher.decryptStr("HnJLf45Y/khsScHzjUXVjQ=="),
                legacyDecode: java.aesBase64DecodeToString(
                    "HnJLf45Y/khsScHzjUXVjQ==",
                    "12345678",
                    "DES/CBC/PKCS5Padding",
                    "abcdefgh"
                )
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "encrypted": "HnJLf45Y/khsScHzjUXVjQ==",
            "decrypted": "hello des",
            "legacyDecode": "hello des"
        })
    );
}

#[test]
fn triple_des_cbc_pkcs5_helpers_match_old_core_key_variants() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var tripleKeyCipher = java.createSymmetricCrypto(
                "DESede/CBC/PKCS5Padding",
                "123456789012345678901234",
                "abcdefgh"
            );
            var twoKeyCipher = createSymmetricCrypto(
                "3DES/CBC/PKCS5Padding",
                "1234567890123456",
                "abcdefgh"
            );
            ({
                tripleKeyEncrypted: tripleKeyCipher.encrypt("hello 3des"),
                tripleKeyDecrypted: tripleKeyCipher.decryptStr("UIFe/3RueSF8qLlQetechQ=="),
                tripleKeyLegacyDecode: java.aesBase64DecodeToString(
                    "UIFe/3RueSF8qLlQetechQ==",
                    "123456789012345678901234",
                    "DESede/CBC/PKCS5Padding",
                    "abcdefgh"
                ),
                twoKeyEncrypted: twoKeyCipher.encrypt("two key 3des"),
                twoKeyDecrypted: twoKeyCipher.decryptStr("7gULg15+QHQIb2Xq0Mm7FA==")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "tripleKeyEncrypted": "UIFe/3RueSF8qLlQetechQ==",
            "tripleKeyDecrypted": "hello 3des",
            "tripleKeyLegacyDecode": "hello 3des",
            "twoKeyEncrypted": "7gULg15+QHQIb2Xq0Mm7FA==",
            "twoKeyDecrypted": "two key 3des"
        })
    );
}

#[test]
fn sm4_helpers_match_old_core_cbc_and_ecb_fixture_paths() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var cbc = java.createSymmetricCrypto(
                "SM4/CBC/PKCS7Padding",
                "0123456789abcdef",
                "abcdefghijklmnop"
            );
            var ecb = createSymmetricCrypto(
                "SM4/ECB/PKCS5Padding",
                "0123456789abcdef"
            );
            ({
                cbcEncrypted: cbc.encrypt("hello sm4 cbc"),
                cbcDecrypted: cbc.decryptStr("3EvR7RLRyhGYn9ZsgqOomQ=="),
                cbcLegacyDecode: java.aesBase64DecodeToString(
                    "3EvR7RLRyhGYn9ZsgqOomQ==",
                    "0123456789abcdef",
                    "SM4/CBC/PKCS7Padding",
                    "abcdefghijklmnop"
                ),
                ecbEncrypted: ecb.encrypt("hello sm4 ecb"),
                ecbDecrypted: ecb.decryptStr("W4J4DEKTXOm0NmlnX0TJ3w==")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "cbcEncrypted": "3EvR7RLRyhGYn9ZsgqOomQ==",
            "cbcDecrypted": "hello sm4 cbc",
            "cbcLegacyDecode": "hello sm4 cbc",
            "ecbEncrypted": "W4J4DEKTXOm0NmlnX0TJ3w==",
            "ecbDecrypted": "hello sm4 ecb"
        })
    );
}

#[test]
fn aes_encode_to_base64_string_alias_matches_guarded_corpus_pattern() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            ({
                javaAlias: java.aesEncodeToBase64String(
                    "42",
                    "12cdefgabcdefg12",
                    "AES/ECB/PKCS5Padding",
                    ""
                ),
                globalAlias: aesEncodeToBase64String(
                    "42",
                    "12cdefgabcdefg12",
                    "AES/ECB/PKCS5Padding",
                    ""
                ),
                factory: java.createSymmetricCrypto(
                    "AES/ECB/PKCS5Padding",
                    "12cdefgabcdefg12",
                    ""
                ).encrypt("42")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "javaAlias": "CVA9zVBOQOAKLEYmEqjFZQ==",
            "globalAlias": "CVA9zVBOQOAKLEYmEqjFZQ==",
            "factory": "CVA9zVBOQOAKLEYmEqjFZQ=="
        })
    );
}

#[test]
fn symmetric_crypto_decrypt_accepts_byte_arrays_for_image_decode_boundary() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var cipher = java.createSymmetricCrypto(
                "AES/CBC/PKCS7Padding",
                "1234567890123456",
                "abcdefghijklmnop"
            );
            var encryptedBytes = java.base64DecodeToByteArray(
                "TV91HgttAaqcCuIa87buLCtI7lGk9I+P7cXIotvFstA="
            );
            var decryptedBytes = cipher.decrypt(encryptedBytes);
            ({
                isArray: Array.isArray(decryptedBytes),
                byteHex: decryptedBytes.map(function(byte) {
                    return byte.toString(16).padStart(2, "0");
                }).join(""),
                text: cipher.decryptStr(encryptedBytes)
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "isArray": true,
            "byteHex": "68656c6c6f2072656164657220636f7265",
            "text": "hello reader core"
        })
    );
}

#[test]
fn buffer_concat_matches_old_image_decode_byte_array_boundary() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var head = [137, 80, 78, 71];
            var tail = [0, 1, 255];
            var merged = Buffer.concat([head, tail]);
            ({
                isArray: Array.isArray(merged),
                length: merged.length,
                hex: merged.map(function(byte) {
                    return byte.toString(16).padStart(2, "0");
                }).join("")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "isArray": true,
            "length": 7,
            "hex": "89504e470001ff"
        })
    );
}

#[test]
fn symmetric_crypto_encrypt_base64_alias_matches_old_core_search_url_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var password = "zc89s30ipHG2Dw";
            var keyStr = password.padEnd(32, "\0");
            var ivStr = password.padEnd(16, "\0");
            var cipher = java.createSymmetricCrypto(
                "AES/CBC/PKCS5Padding",
                keyStr,
                ivStr
            );
            var encrypted = cipher.encryptBase64("reader");
            ({
                encrypted: encrypted,
                encoded: encodeURIComponent(encrypted),
                existingEncrypt: cipher.encrypt("reader")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "encrypted": "1Wun32s7v7oKny6Ki8ugLQ==",
            "encoded": "1Wun32s7v7oKny6Ki8ugLQ%3D%3D",
            "existingEncrypt": "1Wun32s7v7oKny6Ki8ugLQ=="
        })
    );
}

#[test]
fn triple_des_encode_base64_str_matches_old_core_qdsign_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var payload = "Rv1rPTnczce|1700000000||||||";
            var key = "{1dYgqE)h9,R)hKqEcv4]k[h";
            var iv = "01234567";
            ({
                alias: java.tripleDESEncodeBase64Str(
                    payload,
                    key,
                    "CBC",
                    "PKCS5Padding",
                    iv
                ),
                factory: java.createSymmetricCrypto(
                    "DESede/CBC/PKCS5Padding",
                    key,
                    iv
                ).encryptBase64(payload)
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "alias": "R7TCs6Tou2VhUg79AMeC4hxzAyqigNJKgFFKui348Fw=",
            "factory": "R7TCs6Tou2VhUg79AMeC4hxzAyqigNJKgFFKui348Fw="
        })
    );
}

#[test]
fn des_encode_to_base64_string_alias_matches_old_source_yueyou_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var payload = "utId=996ff65deb7fbfc9f9bdcefb6c62f4ce&st=2";
            var key = "snY%169j";
            ({
                alias: java.desEncodeToBase64String(
                    payload,
                    key,
                    "DES/ECB/PKCS5Padding",
                    ""
                ),
                factory: java.createSymmetricCrypto(
                    "DES/ECB/PKCS5Padding",
                    key,
                    ""
                ).encryptBase64(payload)
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "alias": "htmFNli+nnlpn+k/gzU86LNduiGvDrJxFreuHvzy3HqNeq2LzUuaq6JDl5EIzgjS",
            "factory": "htmFNli+nnlpn+k/gzU86LNduiGvDrJxFreuHvzy3HqNeq2LzUuaq6JDl5EIzgjS"
        })
    );
}

#[test]
fn symmetric_crypto_accepts_byte_array_key_and_iv_from_old_core_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var key = java.base64DecodeToByteArray(
                "L6alxSR4ttjXvcGpZozYtdcJtG4l0tSnQplRUONIRsw="
            );
            var iv = java.base64DecodeToByteArray("AAAAAAAAAAAAAAAAAAAAAA==");
            var cipher = java.createSymmetricCrypto(
                "AES/CBC/PKCS5Padding",
                key,
                iv
            );
            var encrypted = cipher.encryptBase64("reader");
            ({
                encrypted: encrypted,
                decrypted: cipher.decryptStr(encrypted),
                keyLength: key.length,
                ivLength: iv.length
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "encrypted": "kg4GcLE/G5RDOo++SL7aZg==",
            "decrypted": "reader",
            "keyLength": 32,
            "ivLength": 16
        })
    );
}

#[test]
fn aes_cbc_zero_padding_matches_old_core_cryptojs_fixture_path() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var cipher = java.createSymmetricCrypto(
                "AES/CBC/ZeroPadding",
                "1234567890123456",
                "abcdefghijklmnop"
            );
            var encrypted = cipher.encryptBase64("reader");
            ({
                encrypted: encrypted,
                decrypted: cipher.decryptStr("jqjb6+d6Y181Yo1RNg7ToA==")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "encrypted": "jqjb6+d6Y181Yo1RNg7ToA==",
            "decrypted": "reader"
        })
    );
}

#[test]
fn symmetric_crypto_encrypt_hex_matches_old_core_object_api_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var cipher = java.createSymmetricCrypto(
                "AES/CBC/PKCS5Padding",
                "1234567890123456",
                "abcdefghijklmnop"
            );
            ({
                hex: cipher.encryptHex("reader"),
                base64: cipher.encryptBase64("reader")
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "hex": "8afb1cbc7a55666b34593192c69ada17",
            "base64": "ivscvHpVZms0WTGSxpraFw=="
        })
    );
}

#[test]
fn symmetric_crypto_decrypt_hex_matches_object_api_fixture() {
    let sandbox = QuickJsSandbox::default();

    let result = sandbox
        .evaluate(
            r#"
            var cipher = java.createSymmetricCrypto(
                "AES/CBC/PKCS5Padding",
                "1234567890123456",
                "abcdefghijklmnop"
            );
            ({
                decrypted: cipher.decryptHex("8afb1cbc7a55666b34593192c69ada17"),
                roundTrip: cipher.decryptHex(cipher.encryptHex("reader"))
            })
            "#,
        )
        .unwrap();

    assert_eq!(
        result.value,
        json!({
            "decrypted": "reader",
            "roundTrip": "reader"
        })
    );
}

//! Host-replay fixture suite for AnalyzeUrl.
//!
//! Loads `tests/fixtures/host_replay/analyze_url_build_suite.json` and
//! replays each case through `AnalyzeUrl::build_request` (or
//! `build_request_with_js` for cases flagged `requires_js`). This is the
//! Legado-parity validation: desensitized patterns modelled on real Legado
//! book source `searchUrl` shapes must build the expected `HostHttpRequest`.
//!
//! Field coverage notes (see fixture JSON for the full matrix):
//! - Supported JSON fields: method, charset, headers, body, retry, js.
//! - Silently dropped (host responsibility / future work): type, webView,
//!   webJs, webViewDelayTime, origin, serverID.
//! - Inline features: @js:, <js>...</js>, {{key}}/{{page}}/pageMinus/pagePlus,
//!   <a,b,c> / <a-b> page list, legacy METHOD,url prefix.

use std::collections::BTreeMap;

use reader_content::analyze_url::{AnalyzeUrl, AnalyzeUrlContext};
use reader_content::RemoteContentPipeline;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
struct BuildSuite {
    cases: Vec<BuildCase>,
}

#[derive(Debug, Deserialize)]
struct BuildCase {
    id: String,
    #[serde(default)]
    #[allow(dead_code)]
    description: String,
    search_url: String,
    #[serde(default)]
    keyword: String,
    #[serde(default)]
    page: u32,
    base_url: String,
    #[serde(default)]
    source_headers: Value,
    #[serde(default)]
    requires_js: bool,
    expected: Expected,
}

#[derive(Debug, Deserialize)]
struct Expected {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    url_starts_with: Option<String>,
    #[serde(default)]
    url_contains: Vec<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    body: Option<Value>,
    /// Substring match against `request.body`. Used when the exact body
    /// depends on encoding behaviour (e.g. `{{key}}` percent-encodes to
    /// UTF-8 regardless of `charset`, so we only assert the prefix).
    #[serde(default)]
    body_contains: Option<String>,
    #[serde(default)]
    charset: Option<String>,
    #[serde(default)]
    content_type_contains: Option<String>,
    #[serde(default)]
    header_contains: BTreeMap<String, String>,
    #[serde(default)]
    header_equals: BTreeMap<String, String>,
    #[serde(default)]
    header_present: Vec<String>,
    #[serde(default)]
    retry_max_attempts: Option<u32>,
}

fn load_suite() -> BuildSuite {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/host_replay/analyze_url_build_suite.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

fn build_request_for_case(case: &BuildCase) -> reader_contract::remote::HostHttpRequest {
    let ctx = AnalyzeUrlContext::for_search(&case.keyword, case.page.max(1));
    let source_headers: Map<String, Value> = match &case.source_headers {
        Value::Object(map) => map.clone(),
        _ => Default::default(),
    };
    if case.requires_js {
        let pipeline = RemoteContentPipeline::new();
        AnalyzeUrl::build_request_with_js(
            &case.search_url,
            &ctx,
            &case.base_url,
            &source_headers,
            |expr, context| pipeline.evaluate_url_js(expr, context),
        )
        .unwrap_or_else(|err| panic!("case {} build_request_with_js failed: {err}", case.id))
    } else {
        AnalyzeUrl::build_request(&case.search_url, &ctx, &case.base_url, &source_headers)
            .unwrap_or_else(|err| panic!("case {} build_request failed: {err}", case.id))
    }
}

fn headers_map(request: &reader_contract::remote::HostHttpRequest) -> Map<String, Value> {
    request
        .headers
        .as_object()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn host_replay_suite_all_cases_build_expected_requests() {
    let suite = load_suite();
    assert!(
        !suite.cases.is_empty(),
        "fixture suite must contain at least one case"
    );

    for case in &suite.cases {
        let request = build_request_for_case(case);
        let headers = headers_map(&request);

        // URL assertions.
        if let Some(expected_url) = &case.expected.url {
            assert_eq!(
                request.url, *expected_url,
                "case {}: url mismatch",
                case.id
            );
        }
        if let Some(prefix) = &case.expected.url_starts_with {
            assert!(
                request.url.starts_with(prefix),
                "case {}: url {} should start with {}",
                case.id,
                request.url,
                prefix
            );
        }
        for fragment in &case.expected.url_contains {
            assert!(
                request.url.contains(fragment),
                "case {}: url {} should contain {}",
                case.id,
                request.url,
                fragment
            );
        }

        // Method.
        if let Some(method) = &case.expected.method {
            assert_eq!(
                request.method, *method,
                "case {}: method mismatch",
                case.id
            );
        }

        // Body.
        if let Some(expected_body) = &case.expected.body {
            match expected_body {
                Value::Null => assert!(
                    request.body.is_none(),
                    "case {}: body should be null",
                    case.id
                ),
                Value::String(s) => assert_eq!(
                    request.body.as_deref(),
                    Some(s.as_str()),
                    "case {}: body mismatch",
                    case.id
                ),
                _ => assert!(
                    request.body.is_some(),
                    "case {}: body should be present",
                    case.id
                ),
            }
        }
        if let Some(fragment) = &case.expected.body_contains {
            let actual = request.body.as_deref().unwrap_or_else(|| {
                panic!(
                    "case {}: body should be present for body_contains check",
                    case.id
                )
            });
            assert!(
                actual.contains(fragment),
                "case {}: body {:?} should contain {:?}",
                case.id,
                actual,
                fragment
            );
        }

        // Charset.
        if let Some(charset) = &case.expected.charset {
            assert_eq!(
                request.charset.as_deref(),
                Some(charset.as_str()),
                "case {}: charset mismatch",
                case.id
            );
        }

        // Content-Type header assertion.
        if let Some(ct_fragment) = &case.expected.content_type_contains {
            let ct = headers
                .get("Content-Type")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!(
                    "case {}: expected Content-Type header but missing",
                    case.id
                ));
            assert!(
                ct.contains(ct_fragment),
                "case {}: Content-Type {} should contain {}",
                case.id,
                ct,
                ct_fragment
            );
        }

        // Header contains (substring).
        for (key, value) in &case.expected.header_contains {
            let actual = headers
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("case {}: header {} missing", case.id, key));
            assert!(
                actual.contains(value),
                "case {}: header {}={:?} should contain {:?}",
                case.id,
                key,
                actual,
                value
            );
        }

        // Header equals (exact).
        for (key, value) in &case.expected.header_equals {
            let actual = headers
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("case {}: header {} missing", case.id, key));
            assert_eq!(
                actual, value,
                "case {}: header {} mismatch",
                case.id, key
            );
        }

        // Header present.
        for key in &case.expected.header_present {
            assert!(
                headers.contains_key(key),
                "case {}: header {} should be present",
                case.id,
                key
            );
        }

        // Retry policy.
        if let Some(max_attempts) = case.expected.retry_max_attempts {
            let retry = request
                .retry
                .as_ref()
                .unwrap_or_else(|| panic!("case {}: retry should be set", case.id));
            assert_eq!(
                retry.max_attempts, max_attempts,
                "case {}: retry.maxAttempts mismatch",
                case.id
            );
        }
    }
}

/// Sanity check: the suite covers the supported field matrix. This is a
/// meta-test that guards against accidentally shrinking the fixture.
#[test]
fn host_replay_suite_covers_supported_field_matrix() {
    let suite = load_suite();
    let ids: Vec<&str> = suite.cases.iter().map(|c| c.id.as_str()).collect();
    // At minimum: plain GET, POST form, GBK charset, custom headers, @js:,
    // <js> block, page-list, retry, DSL js option, relative URL.
    let required = [
        "alpha-plain-get",
        "beta-post-form-body",
        "gamma-post-gbk-charset",
        "delta-get-custom-headers",
        "zeta-at-js-inline",
        "eta-js-tag-block",
        "theta-page-list-single",
        "kappa-post-retry",
        "lambda-dsl-js-option",
        "mu-relative-url",
    ];
    for req in required {
        assert!(
            ids.contains(&req),
            "fixture suite missing required case {req}"
        );
    }
}

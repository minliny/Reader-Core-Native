//! Bridge between `reader-sync` WebDAV descriptors and `reader-contract`
//! host HTTP types.
//!
//! Charter red line 4 (Core/Host boundary): `reader-sync` produces
//! `WebDavRequest` descriptors without knowing how HTTP is executed; this
//! module is the only place that translates those descriptors into
//! `HostHttpRequest` payloads the platform host actually transports, and the
//! only place that translates `HostHttpResponse` back into the
//! `reader-sync`-owned `WebDavResponse`.
//!
//! The bridge is intentionally narrow:
//! - `WebDavRequest.headers` (`Vec<(String, String)>`) → `HostHttpRequest.headers`
//!   (`serde_json::Value` object). Repeated header names are merged into a
//!   comma-joined string, matching typical HTTP semantics for the headers
//!   WebDAV uses (Depth, Content-Type, Authorization).
//! - `WebDavRequest.body` (`Option<Vec<u8>>`) → `HostHttpRequest.body`
//!   (`Option<String>`). WebDAV payloads in this codebase are UTF-8 (JSON
//!   snapshots, XML PROPFIND bodies); non-UTF-8 request bodies are rejected
//!   with a typed error rather than silently lossy-converted. Binary download
//!   is not part of the S5 backup/sync flow.
//! - `WebDavRequest.depth` → injected as the `Depth` request header (per
//!   RFC 4918 §10.6), since `HostHttpRequest` has no first-class depth field.
//! - `WebDavRequest.accepted_status_codes` is NOT forwarded to the host: it is
//!   a Core-side post-condition checked against `WebDavResponse.status` after
//!   the host round-trip completes (see [`webdav_response_check_status`]).
//! - `auth: Option<&str>` is injected as `Authorization` only when provided.
//!   Per the charter, Core never stores plaintext credentials; the caller
//!   supplies a pre-formatted header value (`Basic ...` / `Bearer ...`).
//!
//! The bridge functions are pure (no I/O, no host bus). Wiring them into the
//! runtime dispatch flow (a new `RemoteHostContinuation::WebDav` arm) is
//! deferred to keep this change isolated from concurrent runtime work; the
//! functions are `pub` so the dispatch layer can call them when it lands.

use reader_contract::remote::{HostHttpRequest, HostHttpResponse};
use reader_contract::CoreError;
use reader_sync::webdav_protocol::{WebDavMethod, WebDavRequest, WebDavResponse};

/// Encode `WebDavRequest` as a `HostHttpRequest` ready for `http.execute`
/// dispatch.
///
/// `base_url` is the WebDAV server root (e.g. `https://example.com/dav/`);
/// `req.path` is appended with a single `/` separator if neither side already
/// has one. `auth`, when supplied, must be a complete `Authorization` header
/// value (e.g. `"Basic dXNlcjpwdw=="`); Core never constructs this from raw
/// credentials.
pub fn webdav_request_to_host_http(
    req: &WebDavRequest,
    base_url: &str,
    auth: Option<&str>,
) -> Result<HostHttpRequest, CoreError> {
    if base_url.trim().is_empty() {
        return Err(CoreError::invalid_params(
            "webdav bridge: base_url must not be empty",
        ));
    }
    let url = join_url(base_url, &req.path)?;

    // Build the headers object. Start from the WebDavRequest headers, then
    // inject Depth (if present) and Authorization (if provided). serde_json::Map
    // preserves insertion order on most platforms but header ordering is not
    // semantically meaningful for WebDAV.
    let mut headers = serde_json::Map::new();
    for (name, value) in &req.headers {
        insert_or_merge_header(&mut headers, name, value);
    }
    if let Some(depth) = req.depth {
        // RFC 4918 §10.6: Depth is a single token ("0" | "1" | "infinity").
        // Overwrite any caller-supplied Depth to ensure the descriptor's
        // typed field is authoritative.
        headers.insert(
            "Depth".to_string(),
            serde_json::Value::String(depth.to_string()),
        );
    }
    if let Some(auth_value) = auth {
        if !auth_value.trim().is_empty() {
            insert_or_merge_header(&mut headers, "Authorization", auth_value);
        }
    }

    // Body translation: WebDAV payloads in this codebase are UTF-8 (JSON
    // snapshots, XML PROPFIND). Reject non-UTF-8 bytes with a typed error
    // rather than silently lossy-encoding them.
    let body = match &req.body {
        None => None,
        Some(bytes) if bytes.is_empty() => Some(String::new()),
        Some(bytes) => Some(String::from_utf8(bytes.clone()).map_err(|_| {
            CoreError::invalid_params(
                "webdav bridge: request body is not valid UTF-8; \
                 binary WebDAV payloads are not supported by the host HTTP bridge",
            )
        })?),
    };

    Ok(HostHttpRequest {
        url,
        method: req.method.as_http_verb().to_string(),
        headers: serde_json::Value::Object(headers),
        body,
        charset: None,
        // WebDAV defaults: descriptor layer leaves redirect/retry policy to
        // the host. MKCOL/PUT/DELETE are not safely redirectable per RFC 4918
        // but enforcing that is the host's responsibility.
        follow_redirects: None,
        max_redirects: None,
        retry: None,
        use_platform_cookie_jar: None,
        session: None,
    })
}

/// Decode a `HostHttpResponse` (produced by an `http.execute` completion) into
/// a `WebDavResponse` for `reader-sync` to inspect.
///
/// `status` is required: a missing status is treated as a host contract
/// violation and rejected with a typed error. `body_base64`, when present and
/// non-empty, is decoded and replaces `body` so callers always see the
/// binary-safe bytes (used by hosts that cannot represent binary responses as
/// UTF-8 strings).
pub fn host_http_response_to_webdav(resp: &HostHttpResponse) -> Result<WebDavResponse, CoreError> {
    let status = resp.status.ok_or_else(|| {
        CoreError::invalid_params("webdav bridge: host http response missing status")
    })?;
    if !(100..=599).contains(&status) {
        return Err(CoreError::invalid_params(format!(
            "webdav bridge: host http status out of range: {status}"
        )));
    }
    let headers = resp
        .headers
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));

    // Prefer the base64 payload when present: it is the binary-safe path.
    // Fall back to the string body's bytes. This keeps PROPFIND XML (UTF-8
    // string) and any future binary download on a single code path.
    let body = if let Some(b64) = resp.body_base64.as_deref() {
        if !b64.is_empty() {
            decode_base64(b64)?
        } else {
            resp.body.clone().into_bytes()
        }
    } else {
        resp.body.clone().into_bytes()
    };

    Ok(WebDavResponse {
        status,
        headers,
        body,
        final_url: resp.final_url.clone(),
    })
}

/// Verify that a `WebDavResponse`'s status is in the descriptor's
/// `accepted_status_codes` list. Returns `Ok(())` if accepted (or if the
/// descriptor's list is empty, meaning "accept anything"). On mismatch returns
/// an `invalid_params`-style `CoreError` so the runtime can surface it.
pub fn webdav_response_check_status(
    resp: &WebDavResponse,
    accepted: &[u16],
) -> Result<(), CoreError> {
    if accepted.is_empty() {
        return Ok(());
    }
    if accepted.contains(&resp.status) {
        Ok(())
    } else {
        Err(CoreError::invalid_params(format!(
            "webdav bridge: status {} not in accepted codes {:?}",
            resp.status, accepted
        )))
    }
}

/// Map a `WebDavMethod` to its HTTP verb string. Convenience wrapper around
/// `WebDavMethod::as_http_verb` for call sites that already hold a `&str`
/// method string from a `HostHttpRequest`.
pub fn webdav_method_to_http_verb(method: WebDavMethod) -> &'static str {
    method.as_http_verb()
}

// ----- helpers -----

fn join_url(base_url: &str, path: &str) -> Result<String, CoreError> {
    // Reject paths that look absolute; the bridge expects relative paths
    // resolved against base_url. This keeps the URL composition predictable
    // and prevents a descriptor from accidentally overriding the host.
    if path.starts_with("http://") || path.starts_with("https://") {
        return Err(CoreError::invalid_params(
            "webdav bridge: path must be relative, not an absolute URL",
        ));
    }
    let base = base_url.trim_end_matches('/');
    if path.is_empty() {
        return Ok(base.to_string());
    }
    let tail = if let Some(stripped) = path.strip_prefix('/') {
        stripped
    } else {
        path
    };
    Ok(format!("{base}/{tail}"))
}

fn insert_or_merge_header(
    headers: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    value: &str,
) {
    match headers.get(name) {
        Some(serde_json::Value::String(existing)) => {
            // Merge repeated headers with ", " — matches common HTTP practice.
            headers.insert(
                name.to_string(),
                serde_json::Value::String(format!("{existing}, {value}")),
            );
        }
        _ => {
            headers.insert(
                name.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }
}

/// Minimal RFC 4648 base64 decoder. The workspace does not yet depend on the
/// `base64` crate; rather than adding a supply-chain dependency for a single
/// call site, we implement the standard alphabet. This handles the host
/// `body_base64` field which uses standard (non-URL-safe) base64 with padding.
fn decode_base64(input: &str) -> Result<Vec<u8>, CoreError> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input = input.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }
    if input.len() % 4 != 0 {
        return Err(CoreError::invalid_params(
            "webdav bridge: base64 length not a multiple of 4",
        ));
    }
    let mut lookup = [255u8; 256];
    for (i, &b) in ALPHABET.iter().enumerate() {
        lookup[b as usize] = i as u8;
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i < bytes.len() {
        let chunk = &bytes[i..i + 4];
        let mut vals = [0u8; 4];
        let mut pad = 0u8;
        for (j, &b) in chunk.iter().enumerate() {
            if b == b'=' {
                pad += 1;
                vals[j] = 0;
            } else {
                let v = lookup[b as usize];
                if v == 255 {
                    return Err(CoreError::invalid_params(
                        "webdav bridge: base64 contains non-alphabet byte",
                    ));
                }
                vals[j] = v;
            }
        }
        let triple: u32 = ((vals[0] as u32) << 18)
            | ((vals[1] as u32) << 12)
            | ((vals[2] as u32) << 6)
            | (vals[3] as u32);
        out.push((triple >> 16) as u8);
        if pad < 2 {
            out.push((triple >> 8) as u8);
        }
        if pad < 1 {
            out.push(triple as u8);
        }
        i += 4;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reader_contract::remote::HostHttpResponse;
    use reader_sync::webdav_protocol::{WebDavMethod, WebDavRequest};

    // ----- webdav_request_to_host_http -----

    #[test]
    fn bridge_propfind_injects_depth_header() {
        let req = WebDavRequest::new(WebDavMethod::Propfind, "/books/")
            .with_depth(1)
            .with_header("Content-Type", "application/xml; charset=utf-8");
        let host = webdav_request_to_host_http(&req, "https://dav.example.com/", None).unwrap();
        assert_eq!(host.method, "PROPFIND");
        assert_eq!(host.url, "https://dav.example.com/books/");
        let headers = host.headers.as_object().unwrap();
        assert_eq!(headers.get("Depth").and_then(|v| v.as_str()), Some("1"));
        assert_eq!(
            headers.get("Content-Type").and_then(|v| v.as_str()),
            Some("application/xml; charset=utf-8")
        );
        assert!(host.body.is_none());
        assert!(host.charset.is_none());
    }

    #[test]
    fn bridge_put_utf8_body_uses_plain_string() {
        let body = r#"{"backupId":"b1"}"#.as_bytes().to_vec();
        let req = WebDavRequest::new(WebDavMethod::Put, "/backups/b1.json").with_body(body);
        let host = webdav_request_to_host_http(&req, "https://dav.example.com", None).unwrap();
        assert_eq!(host.method, "PUT");
        assert_eq!(host.url, "https://dav.example.com/backups/b1.json");
        assert_eq!(host.body.as_deref(), Some(r#"{"backupId":"b1"}"#));
        assert!(host.charset.is_none());
    }

    #[test]
    fn bridge_put_binary_body_returns_typed_error() {
        // 0xFF is invalid UTF-8, forcing the error path.
        let body = vec![0xff, 0x00, 0x42, 0xfe];
        let req = WebDavRequest::new(WebDavMethod::Put, "/blob.bin").with_body(body);
        let err = webdav_request_to_host_http(&req, "https://dav.example.com/", None).unwrap_err();
        assert!(err.message.contains("UTF-8"));
    }

    #[test]
    fn bridge_mkcol_has_no_body_no_depth() {
        let req = WebDavRequest::new(WebDavMethod::Mkcol, "/new-dir/");
        let host = webdav_request_to_host_http(&req, "https://dav.example.com", None).unwrap();
        assert_eq!(host.method, "MKCOL");
        assert!(host.body.is_none());
        let headers = host.headers.as_object().unwrap();
        // Depth must not be injected when req.depth is None.
        assert!(!headers.contains_key("Depth"));
    }

    #[test]
    fn bridge_injects_authorization_header_when_auth_provided() {
        let req = WebDavRequest::new(WebDavMethod::Get, "/file.json");
        let host = webdav_request_to_host_http(
            &req,
            "https://dav.example.com",
            Some("Basic dXNlcjpwdw=="),
        )
        .unwrap();
        let headers = host.headers.as_object().unwrap();
        assert_eq!(
            headers.get("Authorization").and_then(|v| v.as_str()),
            Some("Basic dXNlcjpwdw==")
        );
    }

    #[test]
    fn bridge_skips_blank_authorization_header() {
        let req = WebDavRequest::new(WebDavMethod::Get, "/file.json");
        let host =
            webdav_request_to_host_http(&req, "https://dav.example.com", Some("   ")).unwrap();
        let headers = host.headers.as_object().unwrap();
        assert!(!headers.contains_key("Authorization"));
    }

    #[test]
    fn bridge_rejects_empty_base_url() {
        let req = WebDavRequest::new(WebDavMethod::Get, "/file.json");
        let err = webdav_request_to_host_http(&req, "", None).unwrap_err();
        assert!(err.message.contains("base_url"));
    }

    #[test]
    fn bridge_rejects_absolute_path_override() {
        let req = WebDavRequest::new(WebDavMethod::Get, "https://evil.example.com/file");
        let err = webdav_request_to_host_http(&req, "https://dav.example.com", None).unwrap_err();
        assert!(err.message.contains("relative"));
    }

    #[test]
    fn bridge_url_join_handles_missing_trailing_slash() {
        let req = WebDavRequest::new(WebDavMethod::Get, "sub/file.json");
        let host = webdav_request_to_host_http(&req, "https://dav.example.com", None).unwrap();
        assert_eq!(host.url, "https://dav.example.com/sub/file.json");
    }

    #[test]
    fn bridge_url_join_strips_duplicate_slash() {
        let req = WebDavRequest::new(WebDavMethod::Get, "/sub/file.json");
        let host = webdav_request_to_host_http(&req, "https://dav.example.com/", None).unwrap();
        assert_eq!(host.url, "https://dav.example.com/sub/file.json");
    }

    #[test]
    fn bridge_empty_path_returns_base_url() {
        let req = WebDavRequest::new(WebDavMethod::Propfind, "");
        let host = webdav_request_to_host_http(&req, "https://dav.example.com/", None).unwrap();
        assert_eq!(host.url, "https://dav.example.com");
    }

    #[test]
    fn bridge_repeated_headers_merge_with_comma() {
        let req = WebDavRequest::new(WebDavMethod::Get, "/x")
            .with_header("X-Custom", "a")
            .with_header("X-Custom", "b");
        let host = webdav_request_to_host_http(&req, "https://dav.example.com", None).unwrap();
        let headers = host.headers.as_object().unwrap();
        assert_eq!(
            headers.get("X-Custom").and_then(|v| v.as_str()),
            Some("a, b")
        );
    }

    // ----- host_http_response_to_webdav -----

    #[test]
    fn bridge_response_decodes_utf8_body_to_bytes() {
        let resp = HostHttpResponse {
            body: "hello".into(),
            status: Some(200),
            headers: Some(serde_json::json!({"Content-Type": "application/json"})),
            final_url: Some("https://dav.example.com/file.json".into()),
            charset_hint: None,
            body_base64: None,
            session: None,
            redirects: None,
            cookies: None,
        };
        let wd = host_http_response_to_webdav(&resp).unwrap();
        assert_eq!(wd.status, 200);
        assert_eq!(wd.body, b"hello".to_vec());
        assert_eq!(
            wd.headers.get("Content-Type").and_then(|v| v.as_str()),
            Some("application/json")
        );
        assert_eq!(
            wd.final_url.as_deref(),
            Some("https://dav.example.com/file.json")
        );
    }

    #[test]
    fn bridge_response_decodes_base64_body_when_present() {
        // raw bytes that are not valid UTF-8 as a whole
        let raw = vec![0xff, 0x00, 0x42];
        // standard base64 of [0xff, 0x00, 0x42] = "/wBC"
        let encoded = "/wBC";
        let resp = HostHttpResponse {
            body: String::new(),
            status: Some(200),
            headers: None,
            final_url: None,
            charset_hint: None,
            body_base64: Some(encoded.into()),
            session: None,
            redirects: None,
            cookies: None,
        };
        let wd = host_http_response_to_webdav(&resp).unwrap();
        assert_eq!(wd.body, raw);
    }

    #[test]
    fn bridge_response_rejects_missing_status() {
        let resp = HostHttpResponse {
            body: "x".into(),
            status: None,
            headers: None,
            final_url: None,
            charset_hint: None,
            body_base64: None,
            session: None,
            redirects: None,
            cookies: None,
        };
        let err = host_http_response_to_webdav(&resp).unwrap_err();
        assert!(err.message.contains("missing status"));
    }

    #[test]
    fn bridge_response_rejects_status_out_of_range() {
        let resp = HostHttpResponse {
            body: "x".into(),
            status: Some(999),
            headers: None,
            final_url: None,
            charset_hint: None,
            body_base64: None,
            session: None,
            redirects: None,
            cookies: None,
        };
        let err = host_http_response_to_webdav(&resp).unwrap_err();
        assert!(err.message.contains("out of range"));
    }

    #[test]
    fn bridge_response_rejects_malformed_base64() {
        let resp = HostHttpResponse {
            body: String::new(),
            status: Some(200),
            headers: None,
            final_url: None,
            charset_hint: None,
            body_base64: Some("!!!not-base64!!!".into()),
            session: None,
            redirects: None,
            cookies: None,
        };
        let err = host_http_response_to_webdav(&resp).unwrap_err();
        assert!(err.message.contains("base64"));
    }

    // ----- webdav_response_check_status -----

    #[test]
    fn bridge_status_check_empty_accepted_allows_any_status() {
        let resp = WebDavResponse {
            status: 500,
            headers: serde_json::json!({}),
            body: Vec::new(),
            final_url: None,
        };
        assert!(webdav_response_check_status(&resp, &[]).is_ok());
    }

    #[test]
    fn bridge_status_check_match_returns_ok() {
        let resp = WebDavResponse {
            status: 204,
            headers: serde_json::json!({}),
            body: Vec::new(),
            final_url: None,
        };
        assert!(webdav_response_check_status(&resp, &[200, 204]).is_ok());
    }

    #[test]
    fn bridge_status_check_mismatch_returns_err() {
        let resp = WebDavResponse {
            status: 404,
            headers: serde_json::json!({}),
            body: Vec::new(),
            final_url: None,
        };
        let err = webdav_response_check_status(&resp, &[200, 204]).unwrap_err();
        assert!(err.message.contains("404"));
        assert!(err.message.contains("accepted"));
    }

    // ----- round-trip: descriptor → host → response → descriptor -----

    #[test]
    fn bridge_round_trip_propfind_descriptor_survives_host_transport() {
        let req = WebDavRequest::new(WebDavMethod::Propfind, "/books/")
            .with_depth(1)
            .with_header("Content-Type", "application/xml; charset=utf-8")
            .with_accepted_status_codes(vec![207]);
        let host = webdav_request_to_host_http(&req, "https://dav.example.com", None).unwrap();
        assert_eq!(host.method, "PROPFIND");
        assert_eq!(host.url, "https://dav.example.com/books/");

        // Simulate the host echoing a 207 multistatus response.
        let host_resp = HostHttpResponse {
            body: r#"<?xml version="1.0"?><multistatus xmlns="DAV:"/>"#.into(),
            status: Some(207),
            headers: Some(serde_json::json!({"Content-Type": "application/xml; charset=utf-8"})),
            final_url: None,
            charset_hint: None,
            body_base64: None,
            session: None,
            redirects: None,
            cookies: None,
        };
        let wd_resp = host_http_response_to_webdav(&host_resp).unwrap();
        assert_eq!(wd_resp.status, 207);
        assert!(webdav_response_check_status(&wd_resp, &[207]).is_ok());
    }

    #[test]
    fn bridge_method_to_http_verb_covers_all_variants() {
        for (method, verb) in [
            (WebDavMethod::Propfind, "PROPFIND"),
            (WebDavMethod::Get, "GET"),
            (WebDavMethod::Put, "PUT"),
            (WebDavMethod::Delete, "DELETE"),
            (WebDavMethod::Mkcol, "MKCOL"),
            (WebDavMethod::Move, "MOVE"),
            (WebDavMethod::Copy, "COPY"),
        ] {
            assert_eq!(webdav_method_to_http_verb(method), verb);
        }
    }

    // ----- decode_base64 direct coverage -----

    #[test]
    fn decode_base64_handles_padding_variants() {
        // "f" → "Zg==" , "fo" → "Zm8=", "foo" → "Zm9v"
        assert_eq!(decode_base64("Zg==").unwrap(), b"f".to_vec());
        assert_eq!(decode_base64("Zm8=").unwrap(), b"fo".to_vec());
        assert_eq!(decode_base64("Zm9v").unwrap(), b"foo".to_vec());
        // empty input
        assert_eq!(decode_base64("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn decode_base64_rejects_wrong_length() {
        assert!(decode_base64("Zm9").is_err()); // length 3, not multiple of 4
    }

    #[test]
    fn decode_base64_rejects_non_alphabet_chars() {
        assert!(decode_base64("Zm9v!@#$").is_err());
    }
}

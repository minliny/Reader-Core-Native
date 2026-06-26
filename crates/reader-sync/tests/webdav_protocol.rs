//! Tests for WebDAV request/response descriptors and PROPFIND multistatus
//! parsing (reader-sync::webdav_protocol).
//!
//! Baseline alignment:
//! - Swift `URLSessionWebDAVAdapter.swift` (PROPFIND request body lines 90-100,
//!   WebDAVMultistatusParser lines 422-519, element-name normalization 498-501).
//! - Charter red line 4: Core produces descriptors; Host executes HTTP transport.

use reader_sync::webdav_protocol::{
    parse_multistatus, propfind_request_body, WebDavMethod, WebDavRequest, WebDavResource,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// PROPFIND request body (matches Swift URLSessionWebDAVAdapter lines 90-100)
// ---------------------------------------------------------------------------

#[test]
fn propfind_request_body_matches_swift_shape() {
    let body = propfind_request_body();
    assert!(body.contains(r#"<?xml version="1.0" encoding="utf-8"?>"#));
    assert!(body.contains(r#"<propfind xmlns="DAV:">"#));
    assert!(body.contains("<getcontentlength/>"));
    assert!(body.contains("<getlastmodified/>"));
    assert!(body.contains("<getetag/>"));
    assert!(body.contains("<resourcetype/>"));
    assert!(body.contains("</propfind>"));
}

// ---------------------------------------------------------------------------
// WebDavRequest descriptor
// ---------------------------------------------------------------------------

#[test]
fn webdav_request_propfind_carries_depth_and_accepted_status() {
    let req = WebDavRequest::new(WebDavMethod::Propfind, "/reader-core/backups")
        .with_depth(1)
        .with_header("Content-Type", "application/xml; charset=utf-8")
        .with_body(propfind_request_body().as_bytes().to_vec())
        .with_accepted_status_codes(vec![207]);
    assert_eq!(req.method, WebDavMethod::Propfind);
    assert_eq!(req.path, "/reader-core/backups");
    assert_eq!(req.depth, Some(1));
    assert_eq!(req.accepted_status_codes, vec![207]);
    assert!(req
        .headers
        .iter()
        .any(|(k, v)| k == "Content-Type" && v == "application/xml; charset=utf-8"));
    assert!(req.body.is_some());
}

#[test]
fn webdav_request_put_carries_body_no_depth() {
    let req = WebDavRequest::new(WebDavMethod::Put, "/reader-core/backups/b1.json")
        .with_body(b"{\"backup\":\"v1\"}".to_vec())
        .with_header("Content-Type", "application/json; charset=utf-8")
        .with_accepted_status_codes(vec![200, 201, 204]);
    assert_eq!(req.method, WebDavMethod::Put);
    assert_eq!(req.depth, None);
    assert_eq!(req.accepted_status_codes, vec![200, 201, 204]);
}

#[test]
fn webdav_request_mkcol_no_body() {
    let req = WebDavRequest::new(WebDavMethod::Mkcol, "/reader-core/backups")
        .with_accepted_status_codes(vec![200, 201]);
    assert_eq!(req.method, WebDavMethod::Mkcol);
    assert!(req.body.is_none());
}

#[test]
fn webdav_method_as_http_verb() {
    assert_eq!(WebDavMethod::Propfind.as_http_verb(), "PROPFIND");
    assert_eq!(WebDavMethod::Get.as_http_verb(), "GET");
    assert_eq!(WebDavMethod::Put.as_http_verb(), "PUT");
    assert_eq!(WebDavMethod::Delete.as_http_verb(), "DELETE");
    assert_eq!(WebDavMethod::Mkcol.as_http_verb(), "MKCOL");
    assert_eq!(WebDavMethod::Move.as_http_verb(), "MOVE");
    assert_eq!(WebDavMethod::Copy.as_http_verb(), "COPY");
}

// ---------------------------------------------------------------------------
// PROPFIND multistatus parsing (matches Swift WebDAVMultistatusParser)
// ---------------------------------------------------------------------------

#[test]
fn parse_multistatus_single_file_resource() {
    let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/reader-core/backups/b1.json</D:href>
    <D:propstat>
      <D:prop>
        <D:getcontentlength>4096</D:getcontentlength>
        <D:getlastmodified>Mon, 01 Jan 2024 00:00:00 GMT</D:getlastmodified>
        <D:getetag>"etag-b1"</D:getetag>
        <D:resourcetype/>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
    let resources = parse_multistatus(xml).unwrap();
    assert_eq!(resources.len(), 1);
    let r = &resources[0];
    assert_eq!(r.href, "/reader-core/backups/b1.json");
    assert_eq!(r.content_length, Some(4096));
    assert_eq!(
        r.last_modified.as_deref(),
        Some("Mon, 01 Jan 2024 00:00:00 GMT")
    );
    assert_eq!(r.etag.as_deref(), Some("\"etag-b1\""));
    assert!(!r.is_collection);
}

#[test]
fn parse_multistatus_collection_resource() {
    let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/reader-core/backups/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
    let resources = parse_multistatus(xml).unwrap();
    assert_eq!(resources.len(), 1);
    assert!(resources[0].is_collection);
    assert_eq!(resources[0].content_length, None);
    assert_eq!(resources[0].etag, None);
}

#[test]
fn parse_multistatus_multiple_resources() {
    let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/reader-core/backups/b1.json</D:href>
    <D:propstat>
      <D:prop>
        <D:getcontentlength>100</D:getcontentlength>
        <D:resourcetype/>
      </D:prop>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/reader-core/backups/b2.json</D:href>
    <D:propstat>
      <D:prop>
        <D:getcontentlength>200</D:getcontentlength>
        <D:resourcetype/>
      </D:prop>
    </D:propstat>
  </D:response>
</D:multistatus>"#;
    let resources = parse_multistatus(xml).unwrap();
    assert_eq!(resources.len(), 2);
    assert_eq!(resources[0].href, "/reader-core/backups/b1.json");
    assert_eq!(resources[0].content_length, Some(100));
    assert_eq!(resources[1].href, "/reader-core/backups/b2.json");
    assert_eq!(resources[1].content_length, Some(200));
}

#[test]
fn parse_multistatus_handles_lowercase_namespace_prefix() {
    // Swift normalizedElementName strips d:/D: prefix; we must too.
    let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/x/y.json</d:href>
    <d:propstat>
      <d:prop>
        <d:getcontentlength>42</d:getcontentlength>
        <d:resourcetype/>
      </d:prop>
    </d:propstat>
  </d:response>
</d:multistatus>"#;
    let resources = parse_multistatus(xml).unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].href, "/x/y.json");
    assert_eq!(resources[0].content_length, Some(42));
}

#[test]
fn parse_multistatus_handles_no_namespace_prefix() {
    let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<multistatus xmlns="DAV:">
  <response>
    <href>/x/z.json</href>
    <propstat>
      <prop>
        <getcontentlength>7</getcontentlength>
        <resourcetype/>
      </prop>
    </propstat>
  </response>
</multistatus>"#;
    let resources = parse_multistatus(xml).unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].href, "/x/z.json");
    assert_eq!(resources[0].content_length, Some(7));
}

#[test]
fn parse_multistatus_empty_response_list() {
    let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
</D:multistatus>"#;
    let resources = parse_multistatus(xml).unwrap();
    assert!(resources.is_empty());
}

#[test]
fn parse_multistatus_rejects_non_xml_garbage() {
    let err = parse_multistatus(b"not xml at all").unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("multistatus") || msg.contains("XML"), "{msg}");
}

#[test]
fn parse_multistatus_self_closed_collection() {
    // <resourcetype/> alone (no <collection/>) means NOT a collection.
    let xml = br#"<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/a</D:href>
    <D:propstat><D:prop><D:resourcetype/></D:prop></D:propstat>
  </D:response>
</D:multistatus>"#;
    let r = parse_multistatus(xml).unwrap();
    assert!(!r[0].is_collection);
}

// ---------------------------------------------------------------------------
// WebDavResource serde
// ---------------------------------------------------------------------------

#[test]
fn webdav_resource_serde_roundtrip() {
    let r = WebDavResource {
        href: "/p/f.json".into(),
        content_length: Some(99),
        last_modified: Some("Mon, 01 Jan 2024 00:00:00 GMT".into()),
        etag: Some("\"e\"".into()),
        is_collection: false,
    };
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(
        v,
        json!({
            "href": "/p/f.json",
            "contentLength": 99,
            "lastModified": "Mon, 01 Jan 2024 00:00:00 GMT",
            "etag": "\"e\"",
            "isCollection": false
        })
    );
    let back: WebDavResource = serde_json::from_value(v).unwrap();
    assert_eq!(back, r);
}

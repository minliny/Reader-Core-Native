//! High-level WebDAV client operations — each produces a typed
//! [`WebDavRequest`] descriptor, never executing HTTP directly.
//!
//! Aligned against Swift `URLSessionWebDAVAdapter.swift`:
//! - `list_directory` → PROPFIND Depth:1 (lines 89-126).
//! - `download_file` → GET (lines 128-131).
//! - `upload_file` → PUT octet-stream (lines 133-137).
//! - `upload_json` → PUT application/json (lines 271-275).
//! - `delete_file` → DELETE (lines 139-142).
//! - `connection_test` → PROPFIND Depth:0 (lines 144-156).
//!
//! `make_collection` (MKCOL) is NOT in Swift Reader-Core (it only implements
//! PROPFIND/GET/PUT/DELETE) but is needed to create the backup directory on the
//! WebDAV server; per charter red line 3 we补齐 against Legado/standard WebDAV.

use crate::webdav_protocol::{propfind_request_body, WebDavMethod, WebDavRequest};

/// PROPFIND Depth:1 to list a directory's children. Accepted: 207.
pub fn list_directory(path: &str) -> WebDavRequest {
    WebDavRequest::new(WebDavMethod::Propfind, path)
        .with_depth(1)
        .with_header("Content-Type", "application/xml; charset=utf-8")
        .with_body(propfind_request_body().as_bytes().to_vec())
        .with_accepted_status_codes(vec![207])
}

/// GET to download a file's bytes. Accepted: 200.
pub fn download_file(path: &str) -> WebDavRequest {
    WebDavRequest::new(WebDavMethod::Get, path).with_accepted_status_codes(vec![200])
}

/// PUT to upload raw bytes (octet-stream). Accepted: 200, 201, 204.
pub fn upload_file(path: &str, body: Vec<u8>) -> WebDavRequest {
    WebDavRequest::new(WebDavMethod::Put, path)
        .with_header("Content-Type", "application/octet-stream")
        .with_body(body)
        .with_accepted_status_codes(vec![200, 201, 204])
}

/// PUT to upload a JSON document (application/json). Accepted: 200, 201, 204.
/// Used for progress.json and backup {id}.json (matches Swift uploadJSON).
pub fn upload_json(path: &str, json: &str) -> WebDavRequest {
    WebDavRequest::new(WebDavMethod::Put, path)
        .with_header("Content-Type", "application/json; charset=utf-8")
        .with_body(json.as_bytes().to_vec())
        .with_accepted_status_codes(vec![200, 201, 204])
}

/// DELETE a file. Accepted: 200, 204, 404 (idempotent delete).
pub fn delete_file(path: &str) -> WebDavRequest {
    WebDavRequest::new(WebDavMethod::Delete, path).with_accepted_status_codes(vec![200, 204, 404])
}

/// MKCOL to create a collection (directory). Swift Reader-Core lacks this; we
/// add it per charter red line 3 to support creating the backup directory.
/// Accepted: 200, 201, 405 (method not allowed = already exists, tolerated).
pub fn make_collection(path: &str) -> WebDavRequest {
    WebDavRequest::new(WebDavMethod::Mkcol, path).with_accepted_status_codes(vec![200, 201, 405])
}

/// PROPFIND Depth:0 for a connection test (only the status code matters).
/// Accepted: 207.
pub fn connection_test(path: &str) -> WebDavRequest {
    WebDavRequest::new(WebDavMethod::Propfind, path)
        .with_depth(0)
        .with_header("Content-Type", "application/xml; charset=utf-8")
        .with_body(propfind_request_body().as_bytes().to_vec())
        .with_accepted_status_codes(vec![207])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_directory_is_propfind_depth_1() {
        let r = list_directory("/reader-core/backups");
        assert_eq!(r.method, WebDavMethod::Propfind);
        assert_eq!(r.path, "/reader-core/backups");
        assert_eq!(r.depth, Some(1));
        assert_eq!(r.accepted_status_codes, vec![207]);
        assert_eq!(
            r.header("Content-Type"),
            Some("application/xml; charset=utf-8")
        );
        assert!(r.body.is_some());
    }

    #[test]
    fn download_file_is_get() {
        let r = download_file("/a/b.json");
        assert_eq!(r.method, WebDavMethod::Get);
        assert_eq!(r.depth, None);
        assert_eq!(r.accepted_status_codes, vec![200]);
        assert!(r.body.is_none());
    }

    #[test]
    fn upload_file_is_put_octet_stream() {
        let r = upload_file("/a/b.bin", vec![1, 2, 3]);
        assert_eq!(r.method, WebDavMethod::Put);
        assert_eq!(r.body.as_deref(), Some(&[1u8, 2, 3][..]));
        assert_eq!(r.header("Content-Type"), Some("application/octet-stream"));
        assert_eq!(r.accepted_status_codes, vec![200, 201, 204]);
    }

    #[test]
    fn upload_json_is_put_json() {
        let r = upload_json("/p.json", "{\"x\":1}");
        assert_eq!(r.method, WebDavMethod::Put);
        assert_eq!(r.body.as_deref(), Some(b"{\"x\":1}" as &[u8]));
        assert_eq!(
            r.header("Content-Type"),
            Some("application/json; charset=utf-8")
        );
    }

    #[test]
    fn delete_file_accepts_404() {
        let r = delete_file("/gone");
        assert_eq!(r.method, WebDavMethod::Delete);
        assert!(r.accepted_status_codes.contains(&404));
    }

    #[test]
    fn make_collection_is_mkcol_tolerates_405() {
        let r = make_collection("/reader-core/backups");
        assert_eq!(r.method, WebDavMethod::Mkcol);
        assert!(r.body.is_none());
        assert!(r.accepted_status_codes.contains(&405));
    }

    #[test]
    fn connection_test_is_propfind_depth_0() {
        let r = connection_test("/");
        assert_eq!(r.method, WebDavMethod::Propfind);
        assert_eq!(r.depth, Some(0));
        assert_eq!(r.accepted_status_codes, vec![207]);
    }
}

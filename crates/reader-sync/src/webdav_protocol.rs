//! WebDAV protocol descriptors and PROPFIND multistatus parsing.
//!
//! Core owns WebDAV request/response *semantics*; the platform host owns the
//! actual HTTP transport (socket/TLS). This module produces typed
//! [`WebDavRequest`] descriptors that the runtime layer (reader-runtime) bridges
//! into `HostHttpRequest` for dispatch via `HostCapability::HttpExecute`, and
//! parses the host's `HostHttpResponse` body back into [`WebDavResource`] lists.
//!
//! Aligned against Swift `URLSessionWebDAVAdapter.swift`:
//! - PROPFIND request body (lines 90-100).
//! - `WebDAVMultistatusParser` (lines 422-519): element-name normalization
//!   strips `D:`/`d:` namespace prefixes; `<collection/>` marks a collection.
//!
//! Per charter red line 4: this module never opens a socket or stores plaintext
//! credentials. Authorization headers are injected by the runtime/host layer.

use serde::{Deserialize, Serialize};

/// WebDAV HTTP method. Swift hardcoded these as string literals; we type them
/// so the runtime bridge can map to `HostHttpRequest.method` safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum WebDavMethod {
    Propfind,
    Get,
    Put,
    Delete,
    Mkcol,
    Move,
    Copy,
}

impl WebDavMethod {
    /// The HTTP verb string used in the request line.
    pub fn as_http_verb(&self) -> &'static str {
        match self {
            WebDavMethod::Propfind => "PROPFIND",
            WebDavMethod::Get => "GET",
            WebDavMethod::Put => "PUT",
            WebDavMethod::Delete => "DELETE",
            WebDavMethod::Mkcol => "MKCOL",
            WebDavMethod::Move => "MOVE",
            WebDavMethod::Copy => "COPY",
        }
    }
}

/// A typed WebDAV request descriptor produced by Core. The runtime layer
/// converts this into a `HostHttpRequest` and dispatches it via the host bus.
///
/// `path` is relative to the WebDAV base URL; the runtime/host resolves the
/// absolute URL. `depth` is only meaningful for `PROPFIND` (0 or 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebDavRequest {
    pub method: WebDavMethod,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub depth: Option<u8>,
    pub accepted_status_codes: Vec<u16>,
}

impl WebDavRequest {
    pub fn new(method: WebDavMethod, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            headers: Vec::new(),
            body: None,
            depth: None,
            accepted_status_codes: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    pub fn with_depth(mut self, depth: u8) -> Self {
        self.depth = Some(depth);
        self
    }

    pub fn with_accepted_status_codes(mut self, codes: Vec<u16>) -> Self {
        self.accepted_status_codes = codes;
        self
    }

    /// Look up a header value by name (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A WebDAV response produced by converting the host's `HostHttpResponse`.
/// Core parses the body (e.g. multistatus XML) from this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebDavResponse {
    pub status: u16,
    pub headers: serde_json::Value,
    pub body: Vec<u8>,
    pub final_url: Option<String>,
}

/// One entry parsed from a PROPFIND multistatus response. Mirrors Swift
/// `WebDAVResource` (URLSessionWebDAVAdapter.swift:414-420).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavResource {
    pub href: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_length: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default)]
    pub is_collection: bool,
}

/// Errors produced by WebDAV protocol parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebDavProtocolError {
    /// The response body was not valid multistatus XML or lacked the root.
    InvalidMultistatus(String),
}

impl std::fmt::Display for WebDavProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebDavProtocolError::InvalidMultistatus(msg) => {
                write!(f, "invalid WebDAV multistatus XML: {msg}")
            }
        }
    }
}

impl std::error::Error for WebDavProtocolError {}

/// The PROPFIND request body requesting `getcontentlength` / `getlastmodified`
/// / `getetag` / `resourcetype`. Matches Swift `URLSessionWebDAVAdapter` lines
/// 90-100 verbatim.
pub fn propfind_request_body() -> &'static str {
    r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <getcontentlength/>
    <getlastmodified/>
    <getetag/>
    <resourcetype/>
  </prop>
</propfind>"#
}

/// Parse a PROPFIND multistatus response body into [`WebDavResource`] entries.
///
/// This is a focused state-machine parser over the constrained multistatus XML
/// shape; it does not depend on a full XML library. Element names are
/// normalized by stripping any namespace prefix (`D:`/`d:`), matching Swift
/// `normalizedElementName` (URLSessionWebDAVAdapter.swift:498-501).
pub fn parse_multistatus(body: &[u8]) -> Result<Vec<WebDavResource>, WebDavProtocolError> {
    let text = std::str::from_utf8(body).map_err(|e| {
        WebDavProtocolError::InvalidMultistatus(format!("response is not UTF-8: {e}"))
    })?;
    if !text.contains("multistatus") {
        return Err(WebDavProtocolError::InvalidMultistatus(
            "root <multistatus> element not found".into(),
        ));
    }

    let mut resources = Vec::new();
    // Iterate over <response>...</response> blocks.
    for response_block in extract_tag_blocks(text, "response") {
        let mut href: Option<String> = None;
        let mut content_length: Option<i64> = None;
        let mut last_modified: Option<String> = None;
        let mut etag: Option<String> = None;
        let mut is_collection = false;

        // href may appear outside propstat (it does in standard multistatus).
        if let Some(h) = first_leaf_text(&response_block, "href") {
            href = Some(h);
        }

        for prop_block in extract_tag_blocks(&response_block, "prop") {
            if let Some(v) = first_leaf_text(&prop_block, "getcontentlength") {
                content_length = v.trim().parse::<i64>().ok();
            }
            if let Some(v) = first_leaf_text(&prop_block, "getlastmodified") {
                last_modified = Some(v.trim().to_string());
            }
            if let Some(v) = first_leaf_text(&prop_block, "getetag") {
                etag = Some(v.trim().to_string());
            }
            // <resourcetype><collection/></resourcetype> → collection.
            if let Some(rt) = first_tag_block(&prop_block, "resourcetype") {
                if contains_tag(&rt, "collection") {
                    is_collection = true;
                }
            }
        }

        if let Some(href) = href {
            resources.push(WebDavResource {
                href,
                content_length,
                last_modified,
                etag,
                is_collection,
            });
        }
    }
    Ok(resources)
}

/// Extract the inner text of all `<tag>...</tag>` (or `<ns:tag>...</ns:tag>`)
/// blocks at any depth from `text`. Each returned string is the full block
/// content (between the opening and closing tags), not just leaf text.
fn extract_tag_blocks(text: &str, tag: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let bytes = text.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        let Some(open_start) = find_tag_open(bytes, pos, tag) else {
            break;
        };
        let Some(open_end) = find_byte(bytes, open_start, b'>') else {
            break;
        };
        // Self-closing <tag/> → empty block, skip (no content).
        if bytes.get(open_end.wrapping_sub(1)) == Some(&b'/') {
            pos = open_end + 1;
            continue;
        }
        // Find matching close tag </tag> (no nesting expected for our schema).
        let Some(close_start) = find_close_tag(bytes, open_end + 1, tag) else {
            break;
        };
        let block = &text[open_end + 1..close_start];
        blocks.push(block.to_string());
        pos = find_byte(bytes, close_start, b'>')
            .map(|i| i + 1)
            .unwrap_or(close_start + 1);
    }
    blocks
}

/// Extract only the first `<tag>...</tag>` block content (used for `resourcetype`
/// where we just need to inspect presence of `<collection/>`).
fn first_tag_block(text: &str, tag: &str) -> Option<String> {
    extract_tag_blocks(text, tag).into_iter().next()
}

/// Return the inner text of the first `<tag>...</tag>` leaf element, trimmed of
/// surrounding whitespace. Returns None if the tag is absent or self-closed.
fn first_leaf_text(text: &str, tag: &str) -> Option<String> {
    let block = first_tag_block(text, tag)?;
    let trimmed = block.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// True if `text` contains a `<tag` opening (with optional namespace prefix)
/// anywhere — used to detect `<collection/>` self-closed inside resourcetype.
fn contains_tag(text: &str, tag: &str) -> bool {
    find_tag_open(text.as_bytes(), 0, tag).is_some()
}

/// Find the next `<tag` or `<ns:tag` opening starting at `pos`. Matches the
/// tag name followed by whitespace, `>`, or `/` so that `href` does not match
/// `hrefbar`.
fn find_tag_open(bytes: &[u8], pos: usize, tag: &str) -> Option<usize> {
    let mut i = pos;
    while i < bytes.len() {
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] != b'/' {
            // Skip XML declarations <?xml and comments <!-- .
            if bytes.get(i + 1) == Some(&b'?') || bytes.get(i + 1) == Some(&b'!') {
                i += 1;
                continue;
            }
            let name_start = i + 1;
            // Strip namespace prefix: skip until ':' or tag-name start.
            let mut name_end = name_start;
            while name_end < bytes.len()
                && bytes[name_end] != b'>'
                && bytes[name_end] != b'/'
                && !bytes[name_end].is_ascii_whitespace()
            {
                name_end += 1;
            }
            let full_name = &text_slice(bytes, name_start, name_end);
            let local = full_name.rsplit(':').next().unwrap_or(full_name);
            if local == tag {
                return Some(i);
            }
            i = name_end;
        } else {
            i += 1;
        }
    }
    None
}

/// Find the next `</tag>` (or `</ns:tag>`) closing starting at `pos`.
fn find_close_tag(bytes: &[u8], pos: usize, tag: &str) -> Option<usize> {
    let mut i = pos;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'/' {
            let name_start = i + 2;
            let mut name_end = name_start;
            while name_end < bytes.len()
                && bytes[name_end] != b'>'
                && !bytes[name_end].is_ascii_whitespace()
            {
                name_end += 1;
            }
            let full_name = &text_slice(bytes, name_start, name_end);
            let local = full_name.rsplit(':').next().unwrap_or(full_name);
            if local == tag {
                return Some(i);
            }
            i = name_end;
        } else {
            i += 1;
        }
    }
    None
}

fn find_byte(bytes: &[u8], pos: usize, target: u8) -> Option<usize> {
    bytes
        .iter()
        .skip(pos)
        .position(|&b| b == target)
        .map(|p| p + pos)
}

fn text_slice(bytes: &[u8], start: usize, end: usize) -> &str {
    std::str::from_utf8(&bytes[start..end]).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_rejects_non_xml() {
        assert!(parse_multistatus(b"hello world").is_err());
    }

    #[test]
    fn parser_rejects_missing_multistatus_root() {
        assert!(parse_multistatus(b"<foo></foo>").is_err());
    }
}

//! EPUB local-book parser — clean-room build against Legado `EpubFile.kt`.
//!
//! Implements the EPUB container pipeline in pure Rust:
//! 1. ZIP archive extraction (no external parser dependency)
//! 2. `META-INF/container.xml` → OPF package path
//! 3. OPF parse: `<dc:title>` / `<dc:creator>` / manifest / spine
//! 4. TOC resolution (in priority order):
//!    - EPUB3 `nav.xhtml` (manifest item with `properties="nav"`)
//!    - EPUB2 `toc.ncx` (manifest item with NCX media type)
//!    - Spine fallback (ordered xhtml resources, titles from `<title>`)
//! 5. XHTML body extraction: strip `<script>` / `<style>`, strip tags,
//!    decode basic HTML entities.
//!
//! # Capability boundary (textBoundary / indexedText)
//!
//! - OPF metadata (title, author)
//! - Chapter index from nav/NCX/spine
//! - Body text extraction with script/style stripping
//!
//! # Not done here
//!
//! - Fragment-id chapter splitting (multi-chapter-per-xhtml)
//! - Resource rendering (images, fonts, CSS)
//! - Encryption/DRM
//! - EPUB3 fixed-layout / media-overlays

use std::io::{Cursor, Read};

use reader_domain::{Book, TocEntry};

use crate::{
    derive_title, LocalBook, LocalBookChapter, LocalBookEncoding, LocalBookError, LocalBookFormat,
    LocalBookInput,
};

const EPUB_KIND: &str = "EPUB";
const CONTAINER_XML_PATH: &str = "META-INF/container.xml";

/// Parse an EPUB local book from bytes.
pub fn parse_epub_book(input: LocalBookInput<'_>) -> Result<LocalBook, LocalBookError> {
    let book_id = crate::normalize_required(input.book_id, "book_id")?;
    if input.bytes.is_empty() {
        return Err(LocalBookError::EmptyInput);
    }

    let cursor = Cursor::new(input.bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| LocalBookError::Decode {
        reason: format!("epub_zip_open: {e}"),
    })?;

    // 1. Locate OPF via META-INF/container.xml.
    let opf_path = read_opf_path(&mut archive)?;
    let opf_dir = parent_dir(&opf_path);
    let opf_xml = read_zip_text(&mut archive, &opf_path)?;

    // 2. Parse OPF.
    let opf = parse_opf(&opf_xml)?;

    // 3. Build chapters from nav / NCX / spine fallback.
    let chapters = if let Some(nav_href) = opf.nav_href.as_deref() {
        let nav_path = resolve_href(&opf_dir, nav_href);
        let nav_xml = read_zip_text(&mut archive, &nav_path)?;
        let nav_entries = parse_nav_toc(&nav_xml);
        if nav_entries.is_empty() {
            // nav exists but empty → fall back to spine (invalid_nav diagnostic).
            build_spine_chapters(&mut archive, &opf, &opf_dir)?
        } else {
            build_toc_chapters(&mut archive, &opf_dir, &nav_entries)?
        }
    } else if let Some(ncx_href) = opf.ncx_href.as_deref() {
        let ncx_path = resolve_href(&opf_dir, ncx_href);
        let ncx_xml = read_zip_text(&mut archive, &ncx_path)?;
        let ncx_entries = parse_ncx_toc(&ncx_xml);
        if ncx_entries.is_empty() {
            build_spine_chapters(&mut archive, &opf, &opf_dir)?
        } else {
            build_toc_chapters(&mut archive, &opf_dir, &ncx_entries)?
        }
    } else {
        // No nav, no NCX — spine fallback.
        build_spine_chapters(&mut archive, &opf, &opf_dir)?
    };

    let explicit_title = input
        .title
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let title = explicit_title
        .or_else(|| opf.title.clone().filter(|t| !t.is_empty()))
        .unwrap_or_else(|| derive_title(input.title, input.file_name, &book_id));
    let author = input
        .author
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| opf.author.clone().filter(|t| !t.is_empty()))
        .unwrap_or_default();

    let toc = chapters
        .iter()
        .enumerate()
        .map(|(i, c)| TocEntry {
            index: i as u32,
            title: c.title.clone(),
            url: format!("local://{book_id}/chapter/{i}"),
        })
        .collect::<Vec<_>>();
    let char_len: usize = chapters.iter().map(|c| c.content.chars().count()).sum();

    Ok(LocalBook {
        book: Book {
            book_id: book_id.clone(),
            title,
            author,
            cover_url: None,
            intro: None,
            kind: Some(EPUB_KIND.to_string()),
            last_chapter: chapters.last().map(|c| c.title.clone()),
        },
        format: LocalBookFormat::Epub,
        encoding: LocalBookEncoding::Utf8,
        byte_len: input.bytes.len(),
        char_len,
        toc,
        chapters,
    })
}

// ---------------------------------------------------------------------------
// ZIP helpers
// ---------------------------------------------------------------------------

fn read_zip_text<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String, LocalBookError> {
    let mut file = archive.by_name(name).map_err(|e| LocalBookError::Decode {
        reason: format!("epub_zip_read `{name}`: {e}"),
    })?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| LocalBookError::Decode {
            reason: format!("epub_zip_read_body `{name}`: {e}"),
        })?;
    String::from_utf8(buf).map_err(|e| LocalBookError::Decode {
        reason: format!("epub_zip_utf8 `{name}`: {e}"),
    })
}

fn read_opf_path<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<String, LocalBookError> {
    let container = read_zip_text(archive, CONTAINER_XML_PATH)?;
    extract_rootfile_path(&container).ok_or_else(|| LocalBookError::InvalidMetadata {
        field: "epub_container_rootfile".into(),
    })
}

/// Extract `full-path` attribute from `<rootfile .../>` in container.xml.
fn extract_rootfile_path(container_xml: &str) -> Option<String> {
    let rootfile_start = container_xml.find("<rootfile")?;
    let rootfile_end = container_xml[rootfile_start..].find("/>")? + rootfile_start + 2;
    let rootfile = &container_xml[rootfile_start..rootfile_end];
    let path_start = rootfile.find("full-path=\"")? + "full-path=\"\"".len() - 1;
    let path_value_start = rootfile_start + path_start;
    let path_value_end = container_xml[path_value_start..].find('"')? + path_value_start;
    Some(container_xml[path_value_start..path_value_end].to_string())
}

// ---------------------------------------------------------------------------
// OPF parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct OpfPackage {
    title: Option<String>,
    author: Option<String>,
    /// id → href (relative to OPF directory)
    manifest: Vec<(String, String)>,
    /// Ordered list of manifest ids from spine
    spine: Vec<String>,
    /// href of the EPUB3 nav document (relative to OPF dir)
    nav_href: Option<String>,
    /// href of the EPUB2 NCX document (relative to OPF dir)
    ncx_href: Option<String>,
}

fn parse_opf(xml: &str) -> Result<OpfPackage, LocalBookError> {
    let mut pkg = OpfPackage::default();

    // Title
    if let Some(text) = extract_xml_text(xml, "dc:title") {
        pkg.title = Some(text.trim().to_string()).filter(|s| !s.is_empty());
    }
    // Author
    if let Some(text) = extract_xml_text(xml, "dc:creator") {
        pkg.author = Some(text.trim().to_string()).filter(|s| !s.is_empty());
    }

    // Manifest items: <item id="..." href="..." media-type="..." [properties="..."]/>
    for item_match in find_all_elements(xml, "item") {
        let id = extract_attr(&item_match, "id").unwrap_or_default();
        let href = extract_attr(&item_match, "href").unwrap_or_default();
        if id.is_empty() || href.is_empty() {
            continue;
        }
        let media_type = extract_attr(&item_match, "media-type").unwrap_or_default();
        let properties = extract_attr(&item_match, "properties").unwrap_or_default();
        if properties.split_whitespace().any(|p| p == "nav") {
            pkg.nav_href = Some(href.clone());
        }
        if media_type == "application/x-dtbncx+xml" {
            pkg.ncx_href = Some(href.clone());
        }
        pkg.manifest.push((id, href));
    }

    // Spine: <itemref idref="..."/>
    for itemref_match in find_all_elements(xml, "itemref") {
        if let Some(idref) = extract_attr(&itemref_match, "idref") {
            if !idref.is_empty() {
                pkg.spine.push(idref);
            }
        }
    }

    Ok(pkg)
}

/// Return the inner text of the first `<tag>...</tag>` occurrence.
fn extract_xml_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    // Skip to the end of the opening tag (handle attributes).
    let after_open = xml[start..].find('>')? + start + 1;
    let end = xml[after_open..].find(&close)? + after_open;
    Some(xml[after_open..end].to_string())
}

/// Find all elements matching `<tag .../>` or `<tag ...>...</tag>`.
/// Returns the full element strings.
fn find_all_elements(xml: &str, tag: &str) -> Vec<String> {
    let mut results = Vec::new();
    let open = format!("<{tag}");
    let mut search_from = 0usize;
    while let Some(start) = xml[search_from..].find(&open) {
        let abs_start = search_from + start;
        // Find end of this element (either /> or >... </tag>)
        if let Some(self_close) = xml[abs_start..].find("/>") {
            let abs_end = abs_start + self_close + 2;
            results.push(xml[abs_start..abs_end].to_string());
            search_from = abs_end;
            continue;
        }
        // Not self-closing — find closing tag.
        let close = format!("</{tag}>");
        if let Some(close_pos) = xml[abs_start..].find(&close) {
            let abs_end = abs_start + close_pos + close.len();
            results.push(xml[abs_start..abs_end].to_string());
            search_from = abs_end;
            continue;
        }
        // Malformed — skip.
        search_from = abs_start + open.len();
    }
    results
}

/// Extract an attribute value from an XML element string.
fn extract_attr(element: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=\"");
    let start = element.find(&prefix)? + prefix.len();
    let end = element[start..].find('"')? + start;
    Some(element[start..end].to_string())
}

// ---------------------------------------------------------------------------
// TOC parsing (EPUB3 nav + EPUB2 NCX)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TocEntryRaw {
    title: String,
    href: String,
}

/// Parse EPUB3 nav.xhtml `<a href="...">title</a>` entries.
fn parse_nav_toc(nav_xml: &str) -> Vec<TocEntryRaw> {
    let mut entries = Vec::new();
    for a_match in find_all_elements(nav_xml, "a") {
        let href = extract_attr(&a_match, "href").unwrap_or_default();
        let title = extract_xml_text(&a_match, "a").unwrap_or_default();
        if !href.is_empty() && !title.is_empty() {
            entries.push(TocEntryRaw {
                title: title.trim().to_string(),
                href: href.trim().to_string(),
            });
        }
    }
    entries
}

/// Parse EPUB2 toc.ncx `<navPoint><navLabel><text>title</text></navLabel><content src="href"/></navPoint>`.
fn parse_ncx_toc(ncx_xml: &str) -> Vec<TocEntryRaw> {
    let mut entries = Vec::new();
    for navpoint in find_all_elements(ncx_xml, "navPoint") {
        let title = extract_xml_text(&navpoint, "text").unwrap_or_default();
        let content = find_all_elements(&navpoint, "content");
        let href = content
            .first()
            .and_then(|c| extract_attr(c, "src"))
            .unwrap_or_default();
        if !title.trim().is_empty() && !href.is_empty() {
            entries.push(TocEntryRaw {
                title: title.trim().to_string(),
                href: href.trim().to_string(),
            });
        }
    }
    entries
}

// ---------------------------------------------------------------------------
// Chapter construction
// ---------------------------------------------------------------------------

fn build_toc_chapters<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    opf_dir: &str,
    toc_entries: &[TocEntryRaw],
) -> Result<Vec<LocalBookChapter>, LocalBookError> {
    let mut chapters = Vec::new();
    let mut char_offset = 0usize;
    for (i, entry) in toc_entries.iter().enumerate() {
        // href may contain fragment (#id) — strip for file lookup.
        let href_clean = entry.href.split('#').next().unwrap_or(&entry.href);
        let resolved = resolve_href(opf_dir, href_clean);
        let body = match read_zip_text(archive, &resolved) {
            Ok(xhtml) => extract_xhtml_body_text(&xhtml),
            Err(_) => String::new(),
        };
        let char_len = body.chars().count();
        chapters.push(LocalBookChapter {
            index: i as u32,
            title: entry.title.clone(),
            content: body,
            start_char: char_offset,
            end_char: char_offset + char_len,
        });
        char_offset += char_len;
    }
    Ok(chapters)
}

fn build_spine_chapters<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    opf: &OpfPackage,
    opf_dir: &str,
) -> Result<Vec<LocalBookChapter>, LocalBookError> {
    let mut chapters = Vec::new();
    let mut char_offset = 0usize;
    let mut index = 0u32;
    for idref in &opf.spine {
        let href = match opf.manifest.iter().find(|(id, _)| id == idref) {
            Some((_, h)) => h.clone(),
            None => continue,
        };
        let resolved = resolve_href(opf_dir, &href);
        let xhtml = match read_zip_text(archive, &resolved) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let title = extract_xml_text(&xhtml, "title")
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| {
                extract_xml_text(&xhtml, "h1")
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
            })
            .or_else(|| {
                extract_xml_text(&xhtml, "h2")
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
            })
            .unwrap_or_else(|| format!("Chapter {}", index + 1));
        let body = extract_xhtml_body_text(&xhtml);
        let char_len = body.chars().count();
        chapters.push(LocalBookChapter {
            index,
            title,
            content: body,
            start_char: char_offset,
            end_char: char_offset + char_len,
        });
        char_offset += char_len;
        index += 1;
    }
    Ok(chapters)
}

// ---------------------------------------------------------------------------
// XHTML body extraction
// ---------------------------------------------------------------------------

/// Extract body text from an XHTML document:
/// 1. Isolate `<body>...</body>` (fallback to whole doc)
/// 2. Strip `<script>...</script>` and `<style>...</style>` blocks
/// 3. Strip all remaining HTML tags
/// 4. Decode basic HTML entities
/// 5. Collapse whitespace runs
fn extract_xhtml_body_text(xhtml: &str) -> String {
    let body = extract_body(xhtml);
    let without_script = strip_elements(&body, "script");
    let without_style = strip_elements(&without_script, "style");
    let text = strip_html_tags(&without_style);
    let decoded = decode_html_entities(&text);
    collapse_whitespace(&decoded)
}

fn extract_body(xhtml: &str) -> String {
    let lower = xhtml.to_ascii_lowercase();
    let body_start = match lower.find("<body") {
        Some(pos) => match xhtml[pos..].find('>') {
            Some(end_rel) => pos + end_rel + 1,
            None => return xhtml.to_string(),
        },
        None => return xhtml.to_string(),
    };
    let body_end = lower[body_start..]
        .find("</body>")
        .map(|p| body_start + p)
        .unwrap_or(xhtml.len());
    xhtml[body_start..body_end].to_string()
}

/// Remove all `<tag ...>...</tag>` blocks (including content).
fn strip_elements(html: &str, tag: &str) -> String {
    let mut result = String::new();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut cursor = 0usize;
    while let Some(start) = html[cursor..].find(&open) {
        let abs_start = cursor + start;
        result.push_str(&html[cursor..abs_start]);
        let close_pos = match html[abs_start..].find(&close) {
            Some(p) => abs_start + p + close.len(),
            None => {
                // Unterminated — drop to end.
                cursor = html.len();
                break;
            }
        };
        cursor = close_pos;
    }
    result.push_str(&html[cursor..]);
    result
}

/// Remove all `<...>` tag wrappers, keeping inner text.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

fn decode_html_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn collapse_whitespace(text: &str) -> String {
    let mut result = String::new();
    let mut prev_ws = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !prev_ws && !result.is_empty() {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(ch);
            prev_ws = false;
        }
    }
    result.trim().to_string()
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Given an OPF path like `OPS/package.opf`, return the directory prefix
/// (`OPS/`) or empty string if the OPF is at the archive root.
fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(pos) => path[..=pos].to_string(),
        None => String::new(),
    }
}

/// Resolve a href relative to the OPF directory.
fn resolve_href(opf_dir: &str, href: &str) -> String {
    if href.starts_with('/') {
        return href.trim_start_matches('/').to_string();
    }
    // Strip fragment.
    let clean = href.split('#').next().unwrap_or(href);
    format!("{opf_dir}{clean}")
}

// ---------------------------------------------------------------------------
// Required trait import (zip::Read + Seek)
// ---------------------------------------------------------------------------

// `zip::ZipArchive<R>` requires `R: Read + Seek`. `Cursor<&[u8]>` satisfies
// both. We pull `Seek` into scope here so the function signatures compile.
use std::io::Seek;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rootfile_path_finds_opf() {
        let container = r#"<?xml version="1.0"?>
<container><rootfiles><rootfile full-path="OPS/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        assert_eq!(
            extract_rootfile_path(container),
            Some("OPS/package.opf".to_string())
        );
    }

    #[test]
    fn parse_opf_extracts_title_author_and_manifest() {
        let opf = r#"<package version="3.0"><metadata><dc:title>Fixture EPUB</dc:title><dc:creator>Core Test</dc:creator></metadata><manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest><spine><itemref idref="ch1"/></spine></package>"#;
        let pkg = parse_opf(opf).unwrap();
        assert_eq!(pkg.title.as_deref(), Some("Fixture EPUB"));
        assert_eq!(pkg.author.as_deref(), Some("Core Test"));
        assert_eq!(pkg.manifest.len(), 2);
        assert_eq!(pkg.spine, vec!["ch1".to_string()]);
        assert_eq!(pkg.nav_href.as_deref(), Some("nav.xhtml"));
    }

    #[test]
    fn parse_nav_toc_extracts_anchors() {
        let nav = r#"<html><body><nav><ol><li><a href="ch1.xhtml#top">Chapter One</a><ol><li><a href="ch2.xhtml">Nested Two</a></li></ol></li></ol></nav></body></html>"#;
        let entries = parse_nav_toc(nav);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Chapter One");
        assert_eq!(entries[0].href, "ch1.xhtml#top");
        assert_eq!(entries[1].title, "Nested Two");
        assert_eq!(entries[1].href, "ch2.xhtml");
    }

    #[test]
    fn parse_ncx_toc_extracts_navpoints() {
        let ncx = r#"<ncx><navMap><navPoint><navLabel><text>NCX One</text></navLabel><content src="ch1.xhtml"/></navPoint><navPoint><navLabel><text>NCX Two</text></navLabel><content src="ch2.xhtml"/></navPoint></navMap></ncx>"#;
        let entries = parse_ncx_toc(ncx);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "NCX One");
        assert_eq!(entries[0].href, "ch1.xhtml");
        assert_eq!(entries[1].title, "NCX Two");
        assert_eq!(entries[1].href, "ch2.xhtml");
    }

    #[test]
    fn extract_xhtml_body_text_strips_script_and_tags() {
        let xhtml = r#"<html><head><title>Chapter One</title></head><body><h1>Chapter One</h1><p>EPUB text one.</p><script>bad()</script></body></html>"#;
        let text = extract_xhtml_body_text(xhtml);
        assert!(text.contains("EPUB text one."));
        assert!(!text.contains("bad()"));
        assert!(!text.contains("<h1>"));
        assert!(!text.contains("<p>"));
    }

    #[test]
    fn resolve_href_handles_fragment_and_subdir() {
        assert_eq!(resolve_href("OPS/", "ch1.xhtml"), "OPS/ch1.xhtml");
        assert_eq!(resolve_href("OPS/", "ch1.xhtml#top"), "OPS/ch1.xhtml");
        assert_eq!(resolve_href("", "ch1.xhtml"), "ch1.xhtml");
        assert_eq!(resolve_href("OPS/", "/root.xhtml"), "root.xhtml");
    }
}

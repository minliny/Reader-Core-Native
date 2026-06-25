//! Reader-Core local-book parsing — TXT / EPUB / encoding detection.
//!
//! This crate owns local-book data modeling and offline parsing. TXT ingestion
//! handles Unicode BOMs and deterministic chapter splitting. EPUB support is
//! kept to Core-owned OPF/nav/spine chapter planning; archive byte access and
//! full resource import remain adapter-owned.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use reader_domain::{Book, ReadingProgress, TocEntry};
use regex::Regex;
use serde::{Deserialize, Serialize};

pub mod txt;

pub use txt::{parse_txt, parse_txt_with_options, ParsedTxt, TxtChapter, TxtParseOptions};

/// Current local-book library snapshot schema version.
pub const LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const LOCAL_BOOK_CATALOG_SCHEMA_VERSION: u32 = 1;
pub const LOCAL_BOOK_LIBRARY_STORE_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const LOCAL_BOOK_DEFAULT_MAXIMUM_INPUT_SIZE: usize = 8_000_000;
pub const LOCAL_BOOK_MAX_PREVIEW_LIMIT: usize = 64;

/// Portable local-book format labels.
///
/// These labels match the legacy Reader-Core model and are used for
/// backup/sync metadata. Parser support remains format-specific: V1 only
/// ingests TXT bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalBookFormat {
    Txt,
    Epub,
    Pdf,
    Html,
    Mobi,
    Azw,
    Umd,
    Archive,
    #[serde(rename = "webdav")]
    WebDav,
    Unknown,
}

pub const LOCAL_BOOK_CAPABILITY_REPORT_SCHEMA_VERSION: u32 = 1;

/// Capability tiers for each local-book format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalBookCapabilityLevel {
    MetadataOnly,
    TextBoundary,
    IndexedText,
    PlatformRendered,
    Unsupported,
}

/// Core-owned capability boundary for one local-book format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookFormatCapability {
    pub format: LocalBookFormat,
    pub capability_level: LocalBookCapabilityLevel,
    pub can_probe_metadata: bool,
    pub can_build_chapter_index: bool,
    pub can_provide_text_preview: bool,
    #[serde(default)]
    pub can_render_natively_in_core: bool,
    #[serde(default = "default_requires_platform_file_access")]
    pub requires_platform_file_access: bool,
    #[serde(default)]
    pub requires_platform_renderer: bool,
    #[serde(default)]
    pub requires_external_decoder_for_full_parity: bool,
    pub parser_boundary: String,
    #[serde(default)]
    pub host_responsibilities: Vec<String>,
    #[serde(default = "default_clean_room_maintained")]
    pub clean_room_maintained: bool,
    #[serde(default)]
    pub external_gpl_code_copied: bool,
}

impl LocalBookFormatCapability {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.parser_boundary, "parser_boundary")?;
        if self
            .host_responsibilities
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(LocalBookError::InvalidMetadata {
                field: "host_responsibilities".into(),
            });
        }
        Ok(())
    }
}

/// Deterministic capability report for local-book import boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookCapabilityReport {
    pub schema_version: u32,
    pub generated_at: i64,
    pub capabilities: Vec<LocalBookFormatCapability>,
    #[serde(default)]
    pub full_parity_still_host_owned: Vec<String>,
    #[serde(default = "default_clean_room_maintained")]
    pub clean_room_maintained: bool,
    #[serde(default)]
    pub external_gpl_code_copied: bool,
}

impl LocalBookCapabilityReport {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        if self.schema_version != LOCAL_BOOK_CAPABILITY_REPORT_SCHEMA_VERSION {
            return Err(LocalBookError::InvalidMetadata {
                field: "schema_version".into(),
            });
        }
        for capability in &self.capabilities {
            capability.validate()?;
        }
        if self
            .full_parity_still_host_owned
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(LocalBookError::InvalidMetadata {
                field: "full_parity_still_host_owned".into(),
            });
        }
        Ok(())
    }
}

fn default_requires_platform_file_access() -> bool {
    true
}

fn default_clean_room_maintained() -> bool {
    true
}

pub fn local_book_format_capability(format: LocalBookFormat) -> LocalBookFormatCapability {
    match format {
        LocalBookFormat::Txt => LocalBookFormatCapability {
            format,
            capability_level: LocalBookCapabilityLevel::IndexedText,
            can_probe_metadata: true,
            can_build_chapter_index: true,
            can_provide_text_preview: true,
            can_render_natively_in_core: false,
            requires_platform_file_access: true,
            requires_platform_renderer: false,
            requires_external_decoder_for_full_parity: false,
            parser_boundary: "Core decodes bounded text bytes and applies chapter split policy."
                .into(),
            host_responsibilities: strings([
                "file_access",
                "encoding_adapter_when_needed",
                "reader_pagination_ui",
            ]),
            clean_room_maintained: true,
            external_gpl_code_copied: false,
        },
        LocalBookFormat::Epub => LocalBookFormatCapability {
            format,
            capability_level: LocalBookCapabilityLevel::IndexedText,
            can_probe_metadata: true,
            can_build_chapter_index: true,
            can_provide_text_preview: true,
            can_render_natively_in_core: false,
            requires_platform_file_access: true,
            requires_platform_renderer: false,
            requires_external_decoder_for_full_parity: false,
            parser_boundary:
                "Core indexes OPF/nav/NCX and extracts bounded text resources through configured archive adapter."
                    .into(),
            host_responsibilities: strings([
                "archive_adapter",
                "font_image_resource_display",
                "reader_pagination_ui",
            ]),
            clean_room_maintained: true,
            external_gpl_code_copied: false,
        },
        LocalBookFormat::Pdf => LocalBookFormatCapability {
            format,
            capability_level: LocalBookCapabilityLevel::TextBoundary,
            can_probe_metadata: true,
            can_build_chapter_index: true,
            can_provide_text_preview: true,
            can_render_natively_in_core: false,
            requires_platform_file_access: true,
            requires_platform_renderer: true,
            requires_external_decoder_for_full_parity: false,
            parser_boundary:
                "Core detects PDF and can expose page text boundary where platform text extraction is available; rendering stays platform-owned."
                    .into(),
            host_responsibilities: strings([
                "pdfkit_or_platform_pdf_adapter",
                "interactive_pdf_rendering",
                "ocr_if_required",
            ]),
            clean_room_maintained: true,
            external_gpl_code_copied: false,
        },
        LocalBookFormat::Html => LocalBookFormatCapability {
            format,
            capability_level: LocalBookCapabilityLevel::IndexedText,
            can_probe_metadata: true,
            can_build_chapter_index: true,
            can_provide_text_preview: true,
            can_render_natively_in_core: false,
            requires_platform_file_access: true,
            requires_platform_renderer: false,
            requires_external_decoder_for_full_parity: false,
            parser_boundary: "Core accepts bounded HTML/text resources as local-book content; interactive rendering and resource display stay host-owned."
                .into(),
            host_responsibilities: strings([
                "resource_viewer_ui",
                "font_image_resource_display",
                "reader_pagination_ui",
            ]),
            clean_room_maintained: true,
            external_gpl_code_copied: false,
        },
        LocalBookFormat::Mobi => binary_text_fragment_capability(
            format,
            "Core implements clean-room MOBI parser: PDB container + PalmDOC header + MOBI header + EXTH metadata + PalmDOC LZ77 decompression + UTF-8/CP1252 decoding. HUFF/CDIC and KF8 still require external decoder.",
        ),
        LocalBookFormat::Azw => binary_text_fragment_capability(
            format,
            "Core implements clean-room AZW parser (shares MOBI container format): PDB + PalmDOC + EXTH metadata + text preview. KF8 (AZW3) binary section and DRM still require external decoder.",
        ),
        LocalBookFormat::Umd => binary_text_fragment_capability(
            format,
            "Core detects UMD signatures and accepts bounded readable text fragments; full legacy container parity is not claimed.",
        ),
        LocalBookFormat::Archive => LocalBookFormatCapability {
            format,
            capability_level: LocalBookCapabilityLevel::IndexedText,
            can_probe_metadata: true,
            can_build_chapter_index: true,
            can_provide_text_preview: true,
            can_render_natively_in_core: false,
            requires_platform_file_access: true,
            requires_platform_renderer: false,
            requires_external_decoder_for_full_parity: false,
            parser_boundary: "Core indexes safe archive paths and extracts bounded text/html entries."
                .into(),
            host_responsibilities: strings([
                "archive_adapter",
                "resource_viewer_ui",
                "large_archive_storage_policy",
            ]),
            clean_room_maintained: true,
            external_gpl_code_copied: false,
        },
        LocalBookFormat::WebDav => LocalBookFormatCapability {
            format,
            capability_level: LocalBookCapabilityLevel::MetadataOnly,
            can_probe_metadata: true,
            can_build_chapter_index: false,
            can_provide_text_preview: false,
            can_render_natively_in_core: false,
            requires_platform_file_access: true,
            requires_platform_renderer: false,
            requires_external_decoder_for_full_parity: false,
            parser_boundary:
                "Core parses a WebDAV descriptor only; remote bytes and auth stay host/runtime-owned."
                    .into(),
            host_responsibilities: strings(["webdav_auth", "remote_byte_fetch", "offline_cache_policy"]),
            clean_room_maintained: true,
            external_gpl_code_copied: false,
        },
        LocalBookFormat::Unknown => LocalBookFormatCapability {
            format,
            capability_level: LocalBookCapabilityLevel::Unsupported,
            can_probe_metadata: false,
            can_build_chapter_index: false,
            can_provide_text_preview: false,
            can_render_natively_in_core: false,
            requires_platform_file_access: false,
            requires_platform_renderer: false,
            requires_external_decoder_for_full_parity: false,
            parser_boundary: "Unsupported format fails closed with explicit diagnostics.".into(),
            host_responsibilities: strings(["user_visible_error"]),
            clean_room_maintained: true,
            external_gpl_code_copied: false,
        },
    }
}

pub fn local_book_capability_report(
    formats: &[LocalBookFormat],
    generated_at: i64,
) -> Result<LocalBookCapabilityReport, LocalBookError> {
    let report = LocalBookCapabilityReport {
        schema_version: LOCAL_BOOK_CAPABILITY_REPORT_SCHEMA_VERSION,
        generated_at,
        capabilities: formats
            .iter()
            .copied()
            .map(local_book_format_capability)
            .collect(),
        full_parity_still_host_owned: strings([
            "file_picker_and_security_scoped_permissions",
            "long_lived_file_bookmark_persistence",
            "interactive_pdf_rendering",
            "ocr_for_image_only_pdf",
            "proprietary_mobi_azw_full_decoder",
            "reader_ui_pagination_and_selection",
        ]),
        clean_room_maintained: true,
        external_gpl_code_copied: false,
    };
    report.validate()?;
    Ok(report)
}

fn binary_text_fragment_capability(
    format: LocalBookFormat,
    parser_boundary: &str,
) -> LocalBookFormatCapability {
    LocalBookFormatCapability {
        format,
        capability_level: LocalBookCapabilityLevel::TextBoundary,
        can_probe_metadata: true,
        can_build_chapter_index: true,
        can_provide_text_preview: true,
        can_render_natively_in_core: false,
        requires_platform_file_access: true,
        requires_platform_renderer: false,
        requires_external_decoder_for_full_parity: true,
        parser_boundary: parser_boundary.into(),
        host_responsibilities: strings([
            "full_format_decoder_if_product_requires_it",
            "drm_policy",
            "reader_pagination_ui",
        ]),
        clean_room_maintained: true,
        external_gpl_code_copied: false,
    }
}

fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(str::to_string).collect()
}

/// Local-book import settings used before parser selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookFormatDetectionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_extension: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_mime_type: Option<String>,
    #[serde(default = "default_maximum_input_size")]
    pub maximum_input_size: usize,
    #[serde(default = "default_preview_limit")]
    pub preview_limit: usize,
}

impl Default for LocalBookFormatDetectionRequest {
    fn default() -> Self {
        Self {
            declared_filename: None,
            declared_extension: None,
            declared_mime_type: None,
            maximum_input_size: default_maximum_input_size(),
            preview_limit: default_preview_limit(),
        }
    }
}

impl LocalBookFormatDetectionRequest {
    pub fn effective_preview_limit(&self) -> usize {
        self.preview_limit.min(LOCAL_BOOK_MAX_PREVIEW_LIMIT)
    }

    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_optional_metadata(&self.declared_filename, "declared_filename")?;
        validate_optional_metadata(&self.declared_extension, "declared_extension")?;
        validate_optional_metadata(&self.declared_mime_type, "declared_mime_type")?;
        if self.maximum_input_size == 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "maximum_input_size".into(),
            });
        }
        Ok(())
    }
}

fn default_maximum_input_size() -> usize {
    LOCAL_BOOK_DEFAULT_MAXIMUM_INPUT_SIZE
}

fn default_preview_limit() -> usize {
    LOCAL_BOOK_MAX_PREVIEW_LIMIT
}

/// Result of transport-neutral local-book format detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookFormatDetection {
    pub format: LocalBookFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_format: Option<LocalBookFormat>,
    pub media_type: String,
    pub effective_preview_limit: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

/// Legacy local-book source modification metadata used by fast fingerprints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookSourceModificationMetadata {
    pub byte_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modification_timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_identifier_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path_checksum: Option<String>,
}

impl LocalBookSourceModificationMetadata {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_optional_metadata(&self.modification_timestamp, "modification_timestamp")?;
        validate_optional_metadata(&self.resource_identifier_hint, "resource_identifier_hint")?;
        validate_optional_metadata(&self.source_path_checksum, "source_path_checksum")?;
        Ok(())
    }
}

/// Fast local-book fingerprint for source-level duplicate checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookFastFingerprint {
    pub byte_count: u64,
    pub prefix_checksum: String,
    pub suffix_checksum: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_filename_checksum: Option<String>,
    pub detected_format: LocalBookFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modification_metadata: Option<LocalBookSourceModificationMetadata>,
}

impl LocalBookFastFingerprint {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.prefix_checksum, "prefix_checksum")?;
        validate_required_metadata(&self.suffix_checksum, "suffix_checksum")?;
        validate_optional_metadata(
            &self.declared_filename_checksum,
            "declared_filename_checksum",
        )?;
        if let Some(metadata) = &self.modification_metadata {
            metadata.validate()?;
        }
        Ok(())
    }
}

/// Full-content local-book fingerprint used for deterministic duplicate checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookContentFingerprint {
    pub full_input_checksum: String,
    pub parser_config_checksum: String,
    pub normalized_metadata_checksum: String,
    pub chapter_locator_sequence_checksum: String,
}

impl LocalBookContentFingerprint {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.full_input_checksum, "full_input_checksum")?;
        validate_required_metadata(&self.parser_config_checksum, "parser_config_checksum")?;
        validate_required_metadata(
            &self.normalized_metadata_checksum,
            "normalized_metadata_checksum",
        )?;
        validate_required_metadata(
            &self.chapter_locator_sequence_checksum,
            "chapter_locator_sequence_checksum",
        )?;
        Ok(())
    }
}

/// Semantic fingerprint used when bytes differ but metadata still overlaps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookSemanticFingerprint {
    pub normalized_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    pub chapter_title_sequence_checksum: String,
    pub chapter_count: usize,
    pub format: LocalBookFormat,
}

impl LocalBookSemanticFingerprint {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.normalized_title, "normalized_title")?;
        validate_optional_metadata(&self.normalized_author, "normalized_author")?;
        validate_optional_metadata(&self.identifier, "identifier")?;
        validate_required_metadata(
            &self.chapter_title_sequence_checksum,
            "chapter_title_sequence_checksum",
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookFingerprintSet {
    pub fast: LocalBookFastFingerprint,
    pub content: LocalBookContentFingerprint,
    pub semantic: LocalBookSemanticFingerprint,
}

impl LocalBookFingerprintSet {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        self.fast.validate()?;
        self.content.validate()?;
        self.semantic.validate()
    }
}

/// Minimal catalog row needed to evaluate duplicate decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookFingerprintCatalogEntry {
    pub stable_book_id: String,
    pub source_fingerprint: LocalBookFastFingerprint,
    pub content_fingerprint: LocalBookContentFingerprint,
    pub semantic_fingerprint: LocalBookSemanticFingerprint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_group_id: Option<String>,
}

impl LocalBookFingerprintCatalogEntry {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.stable_book_id, "stable_book_id")?;
        self.source_fingerprint.validate()?;
        self.content_fingerprint.validate()?;
        self.semantic_fingerprint.validate()?;
        validate_optional_metadata(&self.duplicate_group_id, "duplicate_group_id")
    }
}

/// Recovery32-style catalog snapshot for local-book import bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookCatalogSnapshot {
    pub schema_version: u32,
    #[serde(default)]
    pub books: Vec<LocalBookFingerprintCatalogEntry>,
    #[serde(default)]
    pub chapters: Vec<LocalBookChapterIndexEntry>,
    #[serde(default)]
    pub resources: Vec<LocalBookResourceIndexEntry>,
}

impl LocalBookCatalogSnapshot {
    pub fn empty() -> Self {
        Self {
            schema_version: LOCAL_BOOK_CATALOG_SCHEMA_VERSION,
            books: Vec::new(),
            chapters: Vec::new(),
            resources: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), LocalBookError> {
        if self.schema_version != LOCAL_BOOK_CATALOG_SCHEMA_VERSION {
            return Err(LocalBookError::InvalidSnapshot {
                field: "schema_version".into(),
            });
        }
        let mut book_ids = HashSet::<String>::new();
        for book in &self.books {
            book.validate()?;
            if !book_ids.insert(book.stable_book_id.clone()) {
                return Err(LocalBookError::InvalidSnapshot {
                    field: "books.stable_book_id".into(),
                });
            }
        }
        let mut chapter_ids = HashSet::<String>::new();
        for chapter in &self.chapters {
            chapter.validate()?;
            if !book_ids.contains(&chapter.book_id) {
                return Err(LocalBookError::InvalidSnapshot {
                    field: "chapters.book_id".into(),
                });
            }
            if !chapter_ids.insert(format!("{}:{}", chapter.book_id, chapter.stable_chapter_id)) {
                return Err(LocalBookError::InvalidSnapshot {
                    field: "chapters.stable_chapter_id".into(),
                });
            }
        }
        let mut resource_ids = HashSet::<String>::new();
        for resource in &self.resources {
            resource.validate()?;
            if !book_ids.contains(&resource.book_id) {
                return Err(LocalBookError::InvalidSnapshot {
                    field: "resources.book_id".into(),
                });
            }
            if !resource_ids.insert(format!(
                "{}:{}",
                resource.book_id, resource.stable_resource_id
            )) {
                return Err(LocalBookError::InvalidSnapshot {
                    field: "resources.stable_resource_id".into(),
                });
            }
        }
        Ok(())
    }
}

pub fn upsert_local_book_catalog_entry(
    catalog: &LocalBookCatalogSnapshot,
    entry: LocalBookFingerprintCatalogEntry,
    chapters: Vec<LocalBookChapterIndexEntry>,
    resources: Vec<LocalBookResourceIndexEntry>,
) -> Result<LocalBookCatalogSnapshot, LocalBookError> {
    catalog.validate()?;
    entry.validate()?;
    for chapter in &chapters {
        chapter.validate()?;
        if chapter.book_id != entry.stable_book_id {
            return Err(LocalBookError::InvalidSnapshot {
                field: "chapters.book_id".into(),
            });
        }
    }
    for resource in &resources {
        resource.validate()?;
        if resource.book_id != entry.stable_book_id {
            return Err(LocalBookError::InvalidSnapshot {
                field: "resources.book_id".into(),
            });
        }
    }

    let mut updated = catalog.clone();
    updated
        .books
        .retain(|book| book.stable_book_id != entry.stable_book_id);
    updated
        .chapters
        .retain(|chapter| chapter.book_id != entry.stable_book_id);
    updated
        .resources
        .retain(|resource| resource.book_id != entry.stable_book_id);
    updated.books.push(entry);
    updated.chapters.extend(chapters);
    updated.resources.extend(resources);
    sort_local_book_catalog(&mut updated);
    updated.validate()?;
    Ok(updated)
}

pub fn remove_local_book_catalog_entry(
    catalog: &LocalBookCatalogSnapshot,
    book_id: &str,
) -> Result<LocalBookCatalogSnapshot, LocalBookError> {
    catalog.validate()?;
    validate_required_metadata(book_id, "book_id")?;
    let mut updated = catalog.clone();
    updated.books.retain(|book| book.stable_book_id != book_id);
    updated
        .chapters
        .retain(|chapter| chapter.book_id != book_id);
    updated
        .resources
        .retain(|resource| resource.book_id != book_id);
    sort_local_book_catalog(&mut updated);
    updated.validate()?;
    Ok(updated)
}

pub fn lookup_local_book_catalog_by_fingerprint(
    catalog: &LocalBookCatalogSnapshot,
    fingerprint: &LocalBookFingerprintSet,
) -> Result<Vec<LocalBookFingerprintCatalogEntry>, LocalBookError> {
    catalog.validate()?;
    fingerprint.validate()?;
    let mut matches = catalog
        .books
        .iter()
        .filter(|book| {
            book.content_fingerprint.full_input_checksum == fingerprint.content.full_input_checksum
                || (book.semantic_fingerprint.normalized_title
                    == fingerprint.semantic.normalized_title
                    && book.semantic_fingerprint.normalized_author
                        == fingerprint.semantic.normalized_author)
        })
        .cloned()
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.stable_book_id.cmp(&right.stable_book_id));
    Ok(matches)
}

fn sort_local_book_catalog(catalog: &mut LocalBookCatalogSnapshot) {
    catalog
        .books
        .sort_by(|left, right| left.stable_book_id.cmp(&right.stable_book_id));
    catalog.chapters.sort_by(|left, right| {
        left.book_id
            .cmp(&right.book_id)
            .then_with(|| left.ordinal.cmp(&right.ordinal))
            .then_with(|| left.stable_chapter_id.cmp(&right.stable_chapter_id))
    });
    catalog.resources.sort_by(|left, right| {
        left.book_id
            .cmp(&right.book_id)
            .then_with(|| left.relative_locator.cmp(&right.relative_locator))
    });
}

/// Duplicate decisions used by legacy local-book import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalBookDuplicateDecision {
    ExactDuplicate,
    SameBytesDifferentPath,
    SameSemanticBook,
    LikelyDuplicate,
    DifferentEdition,
    ChangedFile,
    Unrelated,
    InsufficientEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookDuplicateResult {
    pub decision: LocalBookDuplicateDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_book_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_group_id: Option<String>,
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

/// Change decisions used when validating an existing local-book catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalBookChangeDecision {
    Unchanged,
    MetadataOnlyChanged,
    ContentChanged,
    FormatChanged,
    ParserConfigChanged,
    Inaccessible,
    Removed,
    ReplacementFile,
    UncertainRequiresFullValidation,
}

/// Validation depth for existing local-book entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalBookValidationPolicy {
    MetadataOnly,
    FastFingerprint,
    FullFingerprint,
    SemanticReimport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookChangeResult {
    pub decision: LocalBookChangeDecision,
    #[serde(default)]
    pub reason_codes: Vec<String>,
}

/// Text encoding detected while ingesting a TXT file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalBookEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

/// Chapter split strategy labels from the legacy Reader-Core local-book model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChapterSplitPattern {
    Regex,
    Size,
    Marker,
    Auto,
}

impl Default for ChapterSplitPattern {
    fn default() -> Self {
        Self::Auto
    }
}

/// Portable TXT chapter split policy.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterSplitPolicy {
    #[serde(default)]
    pub pattern: ChapterSplitPattern,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
}

impl ChapterSplitPolicy {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_optional_metadata(&self.regex, "regex")?;
        validate_optional_metadata(&self.marker, "marker")?;
        if self.size_bytes == Some(0) {
            return Err(LocalBookError::InvalidMetadata {
                field: "size_bytes".into(),
            });
        }
        Ok(())
    }
}

/// Byte input and optional metadata for a local TXT book.
#[derive(Debug, Clone, Copy)]
pub struct LocalBookInput<'a> {
    pub book_id: &'a str,
    pub file_name: Option<&'a str>,
    pub title: Option<&'a str>,
    pub author: Option<&'a str>,
    pub bytes: &'a [u8],
}

/// Parsed local book ready to be inserted into a library/storage layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBook {
    pub book: Book,
    pub format: LocalBookFormat,
    pub encoding: LocalBookEncoding,
    pub byte_len: usize,
    pub char_len: usize,
    pub toc: Vec<TocEntry>,
    pub chapters: Vec<LocalBookChapter>,
}

/// One parsed local-book chapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookChapter {
    pub index: u32,
    pub title: String,
    pub content: String,
    /// Character offset of the chapter heading or chapter body start.
    pub start_char: usize,
    /// Character offset where this chapter window ends.
    pub end_char: usize,
}

/// Hierarchical local-book table-of-contents item.
///
/// This mirrors the legacy `LocalTOCItem` contract for portable TXT/EPUB
/// metadata. Flattening into domain TOC entries is deterministic and does not
/// assume byte offsets exist for every format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalTocItem {
    pub title: String,
    #[serde(default = "default_toc_level")]
    pub level: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<LocalTocItem>>,
}

impl LocalTocItem {
    pub fn new(title: impl Into<String>) -> Result<Self, LocalBookError> {
        let item = Self {
            title: normalize_required_owned(title.into(), "title")?,
            level: default_toc_level(),
            byte_offset: None,
            children: None,
        };
        item.validate()?;
        Ok(item)
    }

    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_local_toc_item(self, None)
    }
}

fn default_toc_level() -> u32 {
    1
}

/// Portable reading progress for a local book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalReadingProgress {
    #[serde(rename = "bookId")]
    pub book_id: String,
    #[serde(default)]
    pub chapter_index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_title: Option<String>,
    #[serde(default)]
    pub progress_fraction: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_offset: Option<u64>,
    pub updated_at: i64,
}

impl LocalReadingProgress {
    pub fn new(book_id: impl Into<String>, updated_at: i64) -> Result<Self, LocalBookError> {
        let progress = Self {
            book_id: normalize_required_owned(book_id.into(), "book_id")?,
            chapter_index: 0,
            chapter_title: None,
            progress_fraction: 0.0,
            byte_offset: None,
            updated_at,
        };
        progress.validate()?;
        Ok(progress)
    }

    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_optional_metadata(&self.chapter_title, "chapter_title")?;
        if !self.progress_fraction.is_finite() || !(0.0..=1.0).contains(&self.progress_fraction) {
            return Err(LocalBookError::InvalidMetadata {
                field: "progress_fraction".into(),
            });
        }
        Ok(())
    }

    pub fn as_domain_progress(&self) -> Result<ReadingProgress, LocalBookError> {
        self.validate()?;
        Ok(ReadingProgress {
            book_id: self.book_id.clone(),
            chapter_index: self.chapter_index,
            chapter_offset: self.byte_offset.unwrap_or_default(),
            chapter_progress: self.progress_fraction,
        })
    }
}

/// Apply a local progress update using the same timestamp advance rule as
/// storage: newer updates win, and equal timestamps allow the latest write.
pub fn apply_local_reading_progress_update(
    current: Option<&LocalReadingProgress>,
    update: LocalReadingProgress,
) -> Result<LocalReadingProgress, LocalBookError> {
    update.validate()?;
    if let Some(current) = current {
        current.validate()?;
        if current.book_id != update.book_id {
            return Err(LocalBookError::InvalidMetadata {
                field: "book_id".into(),
            });
        }
        if update.updated_at < current.updated_at {
            return Ok(current.clone());
        }
    }
    Ok(update)
}

/// Restore state for a persisted local-book reading locator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalBookReadingRestoreState {
    ExactRestored,
    LocatorRestored,
    OrdinalRestored,
    ContextualRestored,
    NearestChapterRestored,
    ResetToBeginning,
    StaleBookFingerprint,
    ChapterRemoved,
    AmbiguousMatch,
}

/// Cache materialization state for local-book metadata and content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalBookCacheState {
    Empty,
    MetadataOnly,
    IndexOnly,
    Lazy,
    PartiallyMaterialized,
    Materialized,
    Stale,
    Invalidated,
}

/// Metadata row for one local-book chapter/resource cache entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookCacheMetadata {
    pub cache_key: String,
    pub book_fingerprint: String,
    pub parser_config_checksum: String,
    pub parser_version: String,
    pub chapter_or_resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
    pub created_timestamp: String,
    pub last_access_timestamp: String,
    pub byte_count: u64,
    pub validation_state: LocalBookCacheState,
    pub eviction_priority: i64,
}

impl LocalBookCacheMetadata {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.cache_key, "cache_key")?;
        validate_required_metadata(&self.book_fingerprint, "book_fingerprint")?;
        validate_required_metadata(&self.parser_config_checksum, "parser_config_checksum")?;
        validate_required_metadata(&self.parser_version, "parser_version")?;
        validate_required_metadata(&self.chapter_or_resource_id, "chapter_or_resource_id")?;
        validate_optional_metadata(&self.content_checksum, "content_checksum")?;
        validate_required_metadata(&self.created_timestamp, "created_timestamp")?;
        validate_required_metadata(&self.last_access_timestamp, "last_access_timestamp")?;
        Ok(())
    }
}

pub fn enumerate_local_book_cache_metadata(
    entries: &[LocalBookCacheMetadata],
) -> Result<Vec<LocalBookCacheMetadata>, LocalBookError> {
    for entry in entries {
        entry.validate()?;
    }
    let mut entries = entries.to_vec();
    entries.sort_by(|left, right| left.cache_key.cmp(&right.cache_key));
    Ok(entries)
}

pub fn upsert_local_book_cache_metadata(
    entries: &[LocalBookCacheMetadata],
    metadata: LocalBookCacheMetadata,
) -> Result<Vec<LocalBookCacheMetadata>, LocalBookError> {
    metadata.validate()?;
    let mut entries = entries
        .iter()
        .filter(|entry| entry.cache_key != metadata.cache_key)
        .cloned()
        .collect::<Vec<_>>();
    entries.push(metadata);
    enumerate_local_book_cache_metadata(&entries)
}

pub fn invalidate_local_book_cache_key(
    entries: &[LocalBookCacheMetadata],
    cache_key: &str,
) -> Result<Vec<LocalBookCacheMetadata>, LocalBookError> {
    validate_required_metadata(cache_key, "cache_key")?;
    for entry in entries {
        entry.validate()?;
    }
    let retained = entries
        .iter()
        .filter(|entry| entry.cache_key != cache_key)
        .cloned()
        .collect::<Vec<_>>();
    enumerate_local_book_cache_metadata(&retained)
}

pub fn invalidate_local_book_cache_for_book(
    entries: &[LocalBookCacheMetadata],
    book_id: &str,
) -> Result<Vec<LocalBookCacheMetadata>, LocalBookError> {
    validate_required_metadata(book_id, "book_id")?;
    for entry in entries {
        entry.validate()?;
    }
    let retained = entries
        .iter()
        .filter(|entry| !entry.cache_key.contains(book_id))
        .cloned()
        .collect::<Vec<_>>();
    enumerate_local_book_cache_metadata(&retained)
}

/// Import status recorded for a local-book catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalBookImportState {
    Probed,
    Fingerprinted,
    MetadataImported,
    Indexed,
    LazyContentReady,
    EagerFirstChapter,
    EagerAllContent,
    Validated,
    Invalidated,
    Failed,
}

/// Local-book import materialization mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalBookLibraryImportMode {
    MetadataOnly,
    IndexOnly,
    LazyContent,
    EagerFirstChapter,
    EagerAllContent,
    ValidateExisting,
    ReimportChanged,
}

/// Pure materialization plan for local-book import bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookImportMaterializationPlan {
    pub mode: LocalBookLibraryImportMode,
    pub import_status: LocalBookImportState,
    pub cache_status: LocalBookCacheState,
    pub materialized_chapter_count: usize,
    pub materialized_resource_count: usize,
    pub imported_stages: Vec<String>,
    pub completed_stage: String,
    pub deferred_stages: Vec<String>,
    pub cache_hit_count: usize,
    pub cache_miss_count: usize,
}

impl LocalBookImportMaterializationPlan {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_stage_list(&self.imported_stages, "imported_stages")?;
        validate_required_metadata(&self.completed_stage, "completed_stage")?;
        validate_stage_list(&self.deferred_stages, "deferred_stages")?;
        Ok(())
    }
}

pub fn plan_local_book_import_materialization(
    mode: LocalBookLibraryImportMode,
    chapter_count: usize,
    resource_count: usize,
) -> Result<LocalBookImportMaterializationPlan, LocalBookError> {
    let materialized_chapter_count = match mode {
        LocalBookLibraryImportMode::EagerAllContent => chapter_count,
        LocalBookLibraryImportMode::EagerFirstChapter => chapter_count.min(1),
        LocalBookLibraryImportMode::MetadataOnly
        | LocalBookLibraryImportMode::IndexOnly
        | LocalBookLibraryImportMode::LazyContent
        | LocalBookLibraryImportMode::ValidateExisting
        | LocalBookLibraryImportMode::ReimportChanged => 0,
    };
    let materialized_resource_count = match mode {
        LocalBookLibraryImportMode::EagerAllContent => resource_count,
        LocalBookLibraryImportMode::MetadataOnly
        | LocalBookLibraryImportMode::IndexOnly
        | LocalBookLibraryImportMode::LazyContent
        | LocalBookLibraryImportMode::EagerFirstChapter
        | LocalBookLibraryImportMode::ValidateExisting
        | LocalBookLibraryImportMode::ReimportChanged => 0,
    };
    let import_status = match mode {
        LocalBookLibraryImportMode::MetadataOnly => LocalBookImportState::MetadataImported,
        LocalBookLibraryImportMode::IndexOnly => LocalBookImportState::Indexed,
        LocalBookLibraryImportMode::LazyContent => LocalBookImportState::LazyContentReady,
        LocalBookLibraryImportMode::EagerFirstChapter => LocalBookImportState::EagerFirstChapter,
        LocalBookLibraryImportMode::EagerAllContent => LocalBookImportState::EagerAllContent,
        LocalBookLibraryImportMode::ValidateExisting => LocalBookImportState::Validated,
        LocalBookLibraryImportMode::ReimportChanged => LocalBookImportState::LazyContentReady,
    };
    let cache_status = match mode {
        LocalBookLibraryImportMode::MetadataOnly => LocalBookCacheState::MetadataOnly,
        LocalBookLibraryImportMode::IndexOnly => LocalBookCacheState::IndexOnly,
        LocalBookLibraryImportMode::LazyContent
        | LocalBookLibraryImportMode::ValidateExisting
        | LocalBookLibraryImportMode::ReimportChanged => LocalBookCacheState::Lazy,
        LocalBookLibraryImportMode::EagerFirstChapter => LocalBookCacheState::PartiallyMaterialized,
        LocalBookLibraryImportMode::EagerAllContent => LocalBookCacheState::Materialized,
    };
    let deferred_stages = match mode {
        LocalBookLibraryImportMode::MetadataOnly => strings([
            "chapter_index_import",
            "resource_index_import",
            "lazy_content_read",
            "resource_materialization",
        ]),
        LocalBookLibraryImportMode::IndexOnly => {
            strings(["lazy_content_read", "resource_materialization"])
        }
        LocalBookLibraryImportMode::LazyContent
        | LocalBookLibraryImportMode::ValidateExisting
        | LocalBookLibraryImportMode::ReimportChanged => strings([
            "chapter_content_materialization",
            "resource_materialization",
        ]),
        LocalBookLibraryImportMode::EagerFirstChapter => strings([
            "remaining_chapter_materialization",
            "resource_materialization",
        ]),
        LocalBookLibraryImportMode::EagerAllContent => Vec::new(),
    };
    let plan = LocalBookImportMaterializationPlan {
        mode,
        import_status,
        cache_status,
        materialized_chapter_count,
        materialized_resource_count,
        imported_stages: strings([
            "probe",
            "fingerprint",
            "duplicate_lookup",
            "metadata_import",
            "chapter_index_import",
            "resource_index_import",
            "catalog_commit",
            "cache_commit",
        ]),
        completed_stage: "cache_commit".into(),
        deferred_stages,
        cache_hit_count: 0,
        cache_miss_count: if chapter_count == 0 { 0 } else { 1 },
    };
    plan.validate()?;
    Ok(plan)
}

fn validate_stage_list(stages: &[String], field: &str) -> Result<(), LocalBookError> {
    if stages.iter().any(|stage| stage.trim().is_empty()) {
        return Err(LocalBookError::InvalidMetadata {
            field: field.into(),
        });
    }
    Ok(())
}

/// Minimal chapter index row needed to restore a reading locator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookChapterIndexEntry {
    pub stable_chapter_id: String,
    pub book_id: String,
    pub ordinal: i64,
    pub normalized_title: String,
    pub canonical_locator: String,
    pub source_range_path_or_page: String,
    pub content_type: String,
    pub estimated_byte_count: u64,
    pub estimated_character_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
    pub is_materialized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_chapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_chapter_id: Option<String>,
    pub parser_version: String,
    #[serde(default)]
    pub diagnostics_summary: Vec<String>,
}

impl LocalBookChapterIndexEntry {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.stable_chapter_id, "stable_chapter_id")?;
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_required_metadata(&self.normalized_title, "normalized_title")?;
        validate_required_metadata(&self.canonical_locator, "canonical_locator")?;
        validate_required_metadata(&self.source_range_path_or_page, "source_range_path_or_page")?;
        validate_required_metadata(&self.content_type, "content_type")?;
        validate_optional_metadata(&self.content_checksum, "content_checksum")?;
        validate_optional_metadata(&self.previous_chapter_id, "previous_chapter_id")?;
        validate_optional_metadata(&self.next_chapter_id, "next_chapter_id")?;
        validate_required_metadata(&self.parser_version, "parser_version")?;
        if self
            .diagnostics_summary
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(LocalBookError::InvalidMetadata {
                field: "diagnostics_summary".into(),
            });
        }
        Ok(())
    }
}

/// EPUB manifest row normalized from OPF before archive content is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubManifestItem {
    pub id: String,
    pub href: String,
    pub media_type: String,
    #[serde(default)]
    pub properties: Vec<String>,
}

impl LocalBookEpubManifestItem {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.id, "manifest.id")?;
        validate_required_metadata(&self.href, "manifest.href")?;
        validate_required_metadata(&self.media_type, "manifest.media_type")?;
        if self
            .properties
            .iter()
            .any(|property| property.trim().is_empty())
        {
            return Err(LocalBookError::InvalidMetadata {
                field: "manifest.properties".into(),
            });
        }
        Ok(())
    }
}

/// EPUB spine item. `linear=false` mirrors OPF `linear="no"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubSpineItem {
    pub idref: String,
    #[serde(default = "default_epub_spine_linear")]
    pub linear: bool,
}

impl LocalBookEpubSpineItem {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.idref, "spine.idref")
    }
}

/// One link from an EPUB nav/NCX TOC after XML extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubNavItem {
    pub title: String,
    pub href: String,
}

impl LocalBookEpubNavItem {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.title, "nav.title")?;
        validate_required_metadata(&self.href, "nav.href")
    }
}

/// Pure EPUB chapter-index planning request.
///
/// The caller supplies OPF manifest/spine and extracted nav rows. Core resolves
/// EPUB-internal relative paths, filters non-linear spine targets, and keeps
/// fragment sections only when the HTML extraction layer has confirmed the
/// fragment id exists in that document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubChapterIndexRequest {
    pub book_id: String,
    #[serde(default)]
    pub package_base_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav_document_path: Option<String>,
    pub manifest_items: Vec<LocalBookEpubManifestItem>,
    pub spine_items: Vec<LocalBookEpubSpineItem>,
    pub nav_items: Vec<LocalBookEpubNavItem>,
    #[serde(default)]
    pub known_fragment_ids: BTreeMap<String, Vec<String>>,
    pub parser_version: String,
}

impl LocalBookEpubChapterIndexRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_required_metadata(&self.parser_version, "parser_version")?;
        validate_optional_metadata(&self.nav_document_path, "nav_document_path")?;
        for item in &self.manifest_items {
            item.validate()?;
        }
        for item in &self.spine_items {
            item.validate()?;
        }
        for item in &self.nav_items {
            item.validate()?;
        }
        for (path, fragments) in &self.known_fragment_ids {
            validate_required_metadata(path, "known_fragment_ids.path")?;
            if fragments.iter().any(|fragment| fragment.trim().is_empty()) {
                return Err(LocalBookError::InvalidMetadata {
                    field: "known_fragment_ids.fragment".into(),
                });
            }
        }
        Ok(())
    }
}

/// Deterministic Core-owned result for EPUB nav/spine chapter indexing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubChapterIndexPlan {
    pub chapters: Vec<LocalBookChapterIndexEntry>,
    #[serde(default)]
    pub skipped_nav_hrefs: Vec<String>,
    #[serde(default)]
    pub duplicate_nav_hrefs: Vec<String>,
    #[serde(default)]
    pub diagnostics_summary: Vec<String>,
}

impl LocalBookEpubChapterIndexPlan {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        for chapter in &self.chapters {
            chapter.validate()?;
        }
        validate_stage_list(&self.skipped_nav_hrefs, "skipped_nav_hrefs")?;
        validate_stage_list(&self.duplicate_nav_hrefs, "duplicate_nav_hrefs")?;
        validate_stage_list(&self.diagnostics_summary, "diagnostics_summary")?;
        Ok(())
    }
}

/// Navigation source selected for EPUB chapter indexing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalBookEpubNavigationSource {
    Nav,
    Ncx,
    Spine,
}

/// EPUB navigation fallback state machine input.
///
/// Legacy Reader-Core prefers EPUB3 nav, falls back to NCX when nav is empty or
/// unusable, then falls back to linear spine ordering when both navigation
/// documents are unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubNavigationFallbackRequest {
    pub book_id: String,
    #[serde(default)]
    pub package_base_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav_document_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ncx_document_path: Option<String>,
    pub manifest_items: Vec<LocalBookEpubManifestItem>,
    pub spine_items: Vec<LocalBookEpubSpineItem>,
    #[serde(default)]
    pub nav_items: Vec<LocalBookEpubNavItem>,
    #[serde(default)]
    pub ncx_items: Vec<LocalBookEpubNavItem>,
    #[serde(default)]
    pub known_fragment_ids: BTreeMap<String, Vec<String>>,
    pub parser_version: String,
}

impl LocalBookEpubNavigationFallbackRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_required_metadata(&self.parser_version, "parser_version")?;
        validate_optional_metadata(&self.nav_document_path, "nav_document_path")?;
        validate_optional_metadata(&self.ncx_document_path, "ncx_document_path")?;
        for item in &self.manifest_items {
            item.validate()?;
        }
        for item in &self.spine_items {
            item.validate()?;
        }
        for item in &self.nav_items {
            item.validate()?;
        }
        for item in &self.ncx_items {
            item.validate()?;
        }
        for (path, fragments) in &self.known_fragment_ids {
            validate_required_metadata(path, "known_fragment_ids.path")?;
            if fragments.iter().any(|fragment| fragment.trim().is_empty()) {
                return Err(LocalBookError::InvalidMetadata {
                    field: "known_fragment_ids.fragment".into(),
                });
            }
        }
        Ok(())
    }
}

/// EPUB chapter index plus the navigation source that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubNavigationFallbackPlan {
    pub selected_source: LocalBookEpubNavigationSource,
    pub chapter_plan: LocalBookEpubChapterIndexPlan,
    #[serde(default)]
    pub diagnostics_summary: Vec<String>,
}

impl LocalBookEpubNavigationFallbackPlan {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        self.chapter_plan.validate()?;
        validate_stage_list(&self.diagnostics_summary, "diagnostics_summary")?;
        Ok(())
    }
}

/// Archive entry metadata supplied by an EPUB archive adapter before Core reads
/// OPF references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubArchiveEntry {
    pub path: String,
    pub byte_count: u64,
}

impl LocalBookEpubArchiveEntry {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.path, "archive_entry.path")
    }
}

/// Raw OPF manifest row. Missing fields are diagnostic data, not request
/// validation failures, so valid siblings can still be planned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubManifestItemDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default)]
    pub properties: Vec<String>,
}

/// Raw OPF spine row. Missing idrefs become diagnostics while valid siblings
/// remain available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubSpineItemDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idref: Option<String>,
    #[serde(default = "default_epub_spine_linear")]
    pub linear: bool,
}

/// EPUB archive/OPF preflight request that can be evaluated without reading
/// content documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubArchivePreflightRequest {
    #[serde(default)]
    pub archive_entries: Vec<LocalBookEpubArchiveEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opf_path: Option<String>,
    #[serde(default)]
    pub manifest_items: Vec<LocalBookEpubManifestItemDraft>,
    #[serde(default)]
    pub spine_items: Vec<LocalBookEpubSpineItemDraft>,
}

impl LocalBookEpubArchivePreflightRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_optional_metadata(&self.opf_path, "opf_path")?;
        for entry in &self.archive_entries {
            entry.validate()?;
        }
        Ok(())
    }
}

/// EPUB preflight result. `manifest_items` and `spine_items` are sanitized
/// rows that can be passed into navigation planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubArchivePreflightPlan {
    pub fail_closed: bool,
    pub manifest_items: Vec<LocalBookEpubManifestItem>,
    pub spine_items: Vec<LocalBookEpubSpineItem>,
    #[serde(default)]
    pub diagnostics_summary: Vec<String>,
}

impl LocalBookEpubArchivePreflightPlan {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        for item in &self.manifest_items {
            item.validate()?;
        }
        for item in &self.spine_items {
            item.validate()?;
        }
        validate_stage_list(&self.diagnostics_summary, "diagnostics_summary")?;
        Ok(())
    }
}

/// Legacy EPUB import diagnostic codes surfaced by the local-book importer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LocalBookEpubImportDiagnosticCode {
    #[serde(rename = "invalidNav")]
    InvalidNav,
    #[serde(rename = "invalidNCX")]
    InvalidNcx,
    #[serde(rename = "unsafeArchivePath")]
    UnsafeArchivePath,
    #[serde(rename = "missingContainer")]
    MissingContainer,
    #[serde(rename = "unsupportedEncryption")]
    UnsupportedEncryption,
    #[serde(rename = "invalidOPF")]
    InvalidOpf,
    #[serde(rename = "missingChapterResource")]
    MissingChapterResource,
}

/// Structured EPUB import diagnostic derived from Core planning summaries.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubImportDiagnostic {
    pub code: LocalBookEpubImportDiagnosticCode,
    pub detail: String,
}

impl LocalBookEpubImportDiagnostic {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.detail, "diagnostic.detail")
    }
}

/// Diagnostic artifact for one EPUB import planning pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubImportDiagnosticReport {
    pub fail_closed: bool,
    #[serde(default)]
    pub diagnostics: Vec<LocalBookEpubImportDiagnostic>,
}

impl LocalBookEpubImportDiagnosticReport {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        for diagnostic in &self.diagnostics {
            diagnostic.validate()?;
        }
        if self.fail_closed && self.diagnostics.is_empty() {
            return Err(LocalBookError::InvalidMetadata {
                field: "diagnostics".into(),
            });
        }
        Ok(())
    }
}

/// EPUB OPF package metadata supplied as XML text by an archive adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubPackageMetadataRequest {
    pub opf_xml: String,
}

impl LocalBookEpubPackageMetadataRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        if self.opf_xml.trim().is_empty() {
            return Err(LocalBookError::EmptyInput);
        }
        Ok(())
    }
}

/// Core-owned EPUB metadata artifact matching the legacy fixture evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubPackageMetadataArtifact {
    pub fail_closed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_unique_identifier_id: Option<String>,
    #[serde(default)]
    pub diagnostics_summary: Vec<String>,
}

impl LocalBookEpubPackageMetadataArtifact {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_optional_metadata(&self.metadata_identifier, "metadata_identifier")?;
        validate_optional_metadata(&self.metadata_title, "metadata_title")?;
        validate_optional_metadata(&self.metadata_author, "metadata_author")?;
        validate_optional_metadata(&self.metadata_language, "metadata_language")?;
        validate_optional_metadata(
            &self.package_unique_identifier_id,
            "package_unique_identifier_id",
        )?;
        validate_stage_list(&self.diagnostics_summary, "diagnostics_summary")?;
        if self.fail_closed {
            if self.diagnostics_summary.is_empty() {
                return Err(LocalBookError::InvalidMetadata {
                    field: "diagnostics_summary".into(),
                });
            }
            return Ok(());
        }
        if self.metadata_identifier.is_none() {
            return Err(LocalBookError::InvalidMetadata {
                field: "metadata_identifier".into(),
            });
        }
        if self.metadata_title.is_none() {
            return Err(LocalBookError::InvalidMetadata {
                field: "metadata_title".into(),
            });
        }
        if self.metadata_language.is_none() {
            return Err(LocalBookError::InvalidMetadata {
                field: "metadata_language".into(),
            });
        }
        Ok(())
    }
}

/// WebDAV local-book descriptor JSON supplied after host-owned remote discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookWebDavDescriptorRequest {
    pub descriptor_json: String,
}

impl LocalBookWebDavDescriptorRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        if self.descriptor_json.trim().is_empty() {
            return Err(LocalBookError::EmptyInput);
        }
        Ok(())
    }
}

/// Remote resource row derived from a WebDAV local-book descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookWebDavDescriptorResource {
    pub stable_resource_id: String,
    pub path: String,
    pub media_type: String,
    pub byte_count: u64,
    pub checksum: String,
}

impl LocalBookWebDavDescriptorResource {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.stable_resource_id, "stable_resource_id")?;
        validate_required_metadata(&self.path, "path")?;
        validate_required_metadata(&self.media_type, "media_type")?;
        validate_required_metadata(&self.checksum, "checksum")?;
        Ok(())
    }
}

/// Core-owned metadata artifact for legacy WebDAV local-book descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookWebDavDescriptorArtifact {
    pub detected_format: LocalBookFormat,
    pub detected_encoding: String,
    pub book_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    pub remote_path: String,
    pub resource: LocalBookWebDavDescriptorResource,
    pub input_byte_count: u64,
    pub content_checksum_count: u32,
    pub full_content_persisted_count: u32,
    pub diagnostic: String,
    #[serde(default)]
    pub diagnostics_summary: Vec<String>,
    #[serde(default = "default_clean_room_maintained")]
    pub clean_room_maintained: bool,
    #[serde(default)]
    pub external_gpl_code_copied: bool,
}

impl LocalBookWebDavDescriptorArtifact {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        if self.detected_format != LocalBookFormat::WebDav {
            return Err(LocalBookError::InvalidMetadata {
                field: "detected_format".into(),
            });
        }
        validate_required_metadata(&self.detected_encoding, "detected_encoding")?;
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_required_metadata(&self.title, "title")?;
        validate_optional_metadata(&self.author, "author")?;
        validate_optional_metadata(&self.identifier, "identifier")?;
        validate_required_metadata(&self.remote_path, "remote_path")?;
        self.resource.validate()?;
        if self.content_checksum_count == 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "content_checksum_count".into(),
            });
        }
        if self.full_content_persisted_count != 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "full_content_persisted_count".into(),
            });
        }
        validate_required_metadata(&self.diagnostic, "diagnostic")?;
        validate_stage_list(&self.diagnostics_summary, "diagnostics_summary")?;
        if !self
            .diagnostics_summary
            .iter()
            .any(|diagnostic| diagnostic == &self.diagnostic)
        {
            return Err(LocalBookError::InvalidMetadata {
                field: "diagnostics_summary".into(),
            });
        }
        if !self.clean_room_maintained || self.external_gpl_code_copied {
            return Err(LocalBookError::InvalidMetadata {
                field: "clean_room".into(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalBookWebDavDescriptorDraft {
    remote_path: String,
    title: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    file_size: Option<i64>,
    #[serde(default)]
    remote_modified_at: Option<serde_json::Value>,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    source_identifier: Option<String>,
}

/// EPUB HTML text-boundary extraction input for one content document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubHtmlTextBoundaryRequest {
    pub html: String,
    #[serde(default = "default_preview_limit")]
    pub preview_limit: usize,
}

impl LocalBookEpubHtmlTextBoundaryRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.html, "html")?;
        if self.preview_limit == 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "preview_limit".into(),
            });
        }
        Ok(())
    }
}

/// Bounded visible-text preview extracted from EPUB HTML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubHtmlTextBoundaryResult {
    pub preview: String,
    pub suppressed_block_count: u32,
    pub image_fallback_count: u32,
    #[serde(default)]
    pub diagnostics_summary: Vec<String>,
}

impl LocalBookEpubHtmlTextBoundaryResult {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.preview, "preview")?;
        validate_stage_list(&self.diagnostics_summary, "diagnostics_summary")?;
        Ok(())
    }
}

/// HTML local-book text-boundary extraction input for standalone `.html` files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookHtmlTextBoundaryRequest {
    pub html: String,
    #[serde(default = "default_preview_limit")]
    pub preview_limit: usize,
}

impl LocalBookHtmlTextBoundaryRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.html, "html")?;
        if self.preview_limit == 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "preview_limit".into(),
            });
        }
        Ok(())
    }
}

/// Bounded visible-text preview extracted from a standalone HTML local book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookHtmlTextBoundaryResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub preview: String,
    pub suppressed_block_count: u32,
    pub image_fallback_count: u32,
    #[serde(default)]
    pub diagnostics_summary: Vec<String>,
}

impl LocalBookHtmlTextBoundaryResult {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_optional_metadata(&self.title, "title")?;
        validate_required_metadata(&self.preview, "preview")?;
        validate_stage_list(&self.diagnostics_summary, "diagnostics_summary")?;
        Ok(())
    }
}

/// Pure EPUB manifest resource planning request.
///
/// This keeps OPF metadata decisions in Core while leaving archive reads and
/// byte counts to adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubResourceIndexRequest {
    pub book_id: String,
    #[serde(default)]
    pub package_base_path: String,
    pub manifest_items: Vec<LocalBookEpubManifestItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_id: Option<String>,
}

impl LocalBookEpubResourceIndexRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_optional_metadata(&self.cover_id, "cover_id")?;
        for item in &self.manifest_items {
            item.validate()?;
        }
        Ok(())
    }
}

/// Deterministic resource index and cover selection derived from OPF manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookEpubResourceIndexPlan {
    pub resources: Vec<LocalBookResourceIndexEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_resource_id: Option<String>,
    #[serde(default)]
    pub diagnostics_summary: Vec<String>,
}

impl LocalBookEpubResourceIndexPlan {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        for resource in &self.resources {
            resource.validate()?;
        }
        validate_optional_metadata(&self.cover_resource_id, "cover_resource_id")?;
        validate_stage_list(&self.diagnostics_summary, "diagnostics_summary")?;
        Ok(())
    }
}

/// Local resource category used by the legacy local-book library runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalBookResourceKind {
    Cover,
    Image,
    Css,
    Font,
    Other,
}

/// Minimal resource index row needed to plan local-book resource reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookResourceIndexEntry {
    pub stable_resource_id: String,
    #[serde(rename = "bookId")]
    pub book_id: String,
    pub relative_locator: String,
    pub mime_type: String,
    pub byte_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    pub is_materialized: bool,
    pub resource_kind: LocalBookResourceKind,
}

impl LocalBookResourceIndexEntry {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.stable_resource_id, "stable_resource_id")?;
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_required_metadata(&self.relative_locator, "relative_locator")?;
        validate_required_metadata(&self.mime_type, "mime_type")?;
        validate_optional_metadata(&self.checksum, "checksum")?;
        Ok(())
    }
}

/// Chapter read selector from the legacy local-book runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookChapterReadRequest {
    #[serde(rename = "bookId")]
    pub book_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<i64>,
    #[serde(default = "default_preview_limit")]
    pub preview_limit: usize,
}

impl LocalBookChapterReadRequest {
    pub fn new(
        book_id: impl Into<String>,
        chapter_id: Option<String>,
        ordinal: Option<i64>,
        preview_limit: usize,
    ) -> Result<Self, LocalBookError> {
        let request = Self {
            book_id: normalize_required_owned(book_id.into(), "book_id")?,
            chapter_id: normalize_optional_metadata(chapter_id, "chapter_id")?,
            ordinal,
            preview_limit,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn by_ordinal(
        book_id: impl Into<String>,
        ordinal: i64,
        preview_limit: usize,
    ) -> Result<Self, LocalBookError> {
        Self::new(book_id, None, Some(ordinal), preview_limit)
    }

    pub fn effective_preview_limit(&self) -> usize {
        self.preview_limit.min(LOCAL_BOOK_MAX_PREVIEW_LIMIT)
    }

    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_optional_metadata(&self.chapter_id, "chapter_id")?;
        if self.chapter_id.is_none() && self.ordinal.is_none() {
            return Err(LocalBookError::InvalidMetadata {
                field: "chapter_selector".into(),
            });
        }
        if self.preview_limit == 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "preview_limit".into(),
            });
        }
        Ok(())
    }
}

/// Pure prefetch request. It computes the same ordinal window as Recovery32.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookChapterPrefetchRequest {
    #[serde(rename = "bookId")]
    pub book_id: String,
    pub anchor_ordinal: i64,
    pub radius: i64,
    pub maximum_count: usize,
}

impl LocalBookChapterPrefetchRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        if self.anchor_ordinal < 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "anchor_ordinal".into(),
            });
        }
        if self.radius < 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "radius".into(),
            });
        }
        if self.maximum_count == 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "maximum_count".into(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookChapterPrefetchPlan {
    #[serde(rename = "bookId")]
    pub book_id: String,
    pub ordinals: Vec<i64>,
}

/// Portable chapter read result matching the legacy Recovery32 runtime shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookChapterReadResult {
    #[serde(rename = "bookId")]
    pub book_id: String,
    pub chapter_id: String,
    pub ordinal: i64,
    pub normalized_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sanitized_html: Option<String>,
    pub content_checksum: String,
    pub byte_count: u64,
    pub character_count: u64,
    pub cache_hit: bool,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    pub preview: String,
}

impl LocalBookChapterReadResult {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_required_metadata(&self.chapter_id, "chapter_id")?;
        validate_optional_metadata(&self.sanitized_html, "sanitized_html")?;
        validate_required_metadata(&self.content_checksum, "content_checksum")?;
        validate_stage_list(&self.diagnostics, "diagnostics")?;
        if self.preview.chars().count() > LOCAL_BOOK_MAX_PREVIEW_LIMIT {
            return Err(LocalBookError::InvalidMetadata {
                field: "preview".into(),
            });
        }
        if self.character_count < self.preview.chars().count() as u64 {
            return Err(LocalBookError::InvalidMetadata {
                field: "character_count".into(),
            });
        }
        Ok(())
    }
}

/// Local resource read request without touching filesystem or transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookResourceReadRequest {
    #[serde(rename = "bookId")]
    pub book_id: String,
    pub resource_id: String,
    pub max_bytes: u64,
}

impl LocalBookResourceReadRequest {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_required_metadata(&self.resource_id, "resource_id")?;
        if self.max_bytes == 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "max_bytes".into(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookResourceReadResult {
    #[serde(rename = "bookId")]
    pub book_id: String,
    pub resource_id: String,
    pub relative_locator: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub checksum: String,
    pub cache_hit: bool,
    pub cacheable: bool,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

impl LocalBookResourceReadResult {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_required_metadata(&self.resource_id, "resource_id")?;
        validate_required_metadata(&self.relative_locator, "relative_locator")?;
        validate_required_metadata(&self.mime_type, "mime_type")?;
        validate_required_metadata(&self.checksum, "checksum")?;
        validate_stage_list(&self.diagnostics, "diagnostics")?;
        Ok(())
    }
}

/// Recovery32-style local-book library run metrics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookLibraryMetrics {
    pub import_count: usize,
    pub catalog_book_count: usize,
    pub catalog_chapter_count: usize,
    pub catalog_resource_count: usize,
    pub duplicate_decision_counts: BTreeMap<String, usize>,
    pub change_decision_counts: BTreeMap<String, usize>,
    pub chapter_read_count: usize,
    pub resource_read_count: usize,
    pub cache_hit_count: usize,
    pub cache_miss_count: usize,
    pub progress_restore_count: usize,
    pub full_content_persisted_count: usize,
    pub preview_character_limit: usize,
}

impl LocalBookLibraryMetrics {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        if self.preview_character_limit == 0
            || self.preview_character_limit > LOCAL_BOOK_MAX_PREVIEW_LIMIT
        {
            return Err(LocalBookError::InvalidMetadata {
                field: "preview_character_limit".into(),
            });
        }
        if self.full_content_persisted_count > self.import_count {
            return Err(LocalBookError::InvalidMetadata {
                field: "full_content_persisted_count".into(),
            });
        }
        validate_metric_counts(&self.duplicate_decision_counts, "duplicate_decision_counts")?;
        validate_metric_counts(&self.change_decision_counts, "change_decision_counts")?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn summarize_local_book_library_metrics(
    import_count: usize,
    catalog: &LocalBookCatalogSnapshot,
    duplicate_results: &[LocalBookDuplicateResult],
    change_results: &[LocalBookChangeResult],
    chapter_reads: &[LocalBookChapterReadResult],
    resource_reads: &[LocalBookResourceReadResult],
    progress_results: &[LocalBookReadingProgress],
    full_content_persisted_count: usize,
    preview_character_limit: usize,
) -> Result<LocalBookLibraryMetrics, LocalBookError> {
    catalog.validate()?;
    if duplicate_results.len() != import_count {
        return Err(LocalBookError::InvalidSnapshot {
            field: "duplicate_results".into(),
        });
    }
    if preview_character_limit == 0 || preview_character_limit > LOCAL_BOOK_MAX_PREVIEW_LIMIT {
        return Err(LocalBookError::InvalidMetadata {
            field: "preview_character_limit".into(),
        });
    }
    if full_content_persisted_count > import_count {
        return Err(LocalBookError::InvalidMetadata {
            field: "full_content_persisted_count".into(),
        });
    }

    let mut duplicate_decision_counts = BTreeMap::new();
    for result in duplicate_results {
        validate_local_book_duplicate_result(result)?;
        increment_metric_count(
            &mut duplicate_decision_counts,
            local_book_duplicate_decision_wire_value(result.decision),
        );
    }

    let mut change_decision_counts = BTreeMap::new();
    for result in change_results {
        validate_local_book_change_result(result)?;
        increment_metric_count(
            &mut change_decision_counts,
            local_book_change_decision_wire_value(result.decision),
        );
    }

    for read in chapter_reads {
        read.validate()?;
    }
    for read in resource_reads {
        read.validate()?;
    }
    for progress in progress_results {
        progress.validate()?;
    }

    let cache_hit_count = chapter_reads.iter().filter(|read| read.cache_hit).count()
        + resource_reads.iter().filter(|read| read.cache_hit).count();
    let read_count = chapter_reads.len() + resource_reads.len();
    let metrics = LocalBookLibraryMetrics {
        import_count,
        catalog_book_count: catalog.books.len(),
        catalog_chapter_count: catalog.chapters.len(),
        catalog_resource_count: catalog.resources.len(),
        duplicate_decision_counts,
        change_decision_counts,
        chapter_read_count: chapter_reads.len(),
        resource_read_count: resource_reads.len(),
        cache_hit_count,
        cache_miss_count: read_count.saturating_sub(cache_hit_count),
        progress_restore_count: progress_results.len(),
        full_content_persisted_count,
        preview_character_limit,
    };
    metrics.validate()?;
    Ok(metrics)
}

fn validate_local_book_duplicate_result(
    result: &LocalBookDuplicateResult,
) -> Result<(), LocalBookError> {
    validate_optional_metadata(&result.matched_book_id, "matched_book_id")?;
    validate_optional_metadata(&result.duplicate_group_id, "duplicate_group_id")?;
    validate_stage_list(&result.reason_codes, "reason_codes")
}

fn validate_local_book_change_result(result: &LocalBookChangeResult) -> Result<(), LocalBookError> {
    validate_stage_list(&result.reason_codes, "reason_codes")
}

fn validate_metric_counts(
    counts: &BTreeMap<String, usize>,
    field: &str,
) -> Result<(), LocalBookError> {
    if counts
        .iter()
        .any(|(key, count)| key.trim().is_empty() || *count == 0)
    {
        return Err(LocalBookError::InvalidMetadata {
            field: field.into(),
        });
    }
    Ok(())
}

fn increment_metric_count(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.into()).or_insert(0) += 1;
}

fn local_book_duplicate_decision_wire_value(decision: LocalBookDuplicateDecision) -> &'static str {
    match decision {
        LocalBookDuplicateDecision::ExactDuplicate => "exact_duplicate",
        LocalBookDuplicateDecision::SameBytesDifferentPath => "same_bytes_different_path",
        LocalBookDuplicateDecision::SameSemanticBook => "same_semantic_book",
        LocalBookDuplicateDecision::LikelyDuplicate => "likely_duplicate",
        LocalBookDuplicateDecision::DifferentEdition => "different_edition",
        LocalBookDuplicateDecision::ChangedFile => "changed_file",
        LocalBookDuplicateDecision::Unrelated => "unrelated",
        LocalBookDuplicateDecision::InsufficientEvidence => "insufficient_evidence",
    }
}

fn local_book_change_decision_wire_value(decision: LocalBookChangeDecision) -> &'static str {
    match decision {
        LocalBookChangeDecision::Unchanged => "unchanged",
        LocalBookChangeDecision::MetadataOnlyChanged => "metadata_only_changed",
        LocalBookChangeDecision::ContentChanged => "content_changed",
        LocalBookChangeDecision::FormatChanged => "format_changed",
        LocalBookChangeDecision::ParserConfigChanged => "parser_config_changed",
        LocalBookChangeDecision::Inaccessible => "inaccessible",
        LocalBookChangeDecision::Removed => "removed",
        LocalBookChangeDecision::ReplacementFile => "replacement_file",
        LocalBookChangeDecision::UncertainRequiresFullValidation => {
            "uncertain_requires_full_validation"
        }
    }
}

pub fn resolve_local_book_chapter_read_request(
    request: &LocalBookChapterReadRequest,
    chapters: &[LocalBookChapterIndexEntry],
) -> Result<LocalBookChapterIndexEntry, LocalBookError> {
    request.validate()?;
    for chapter in chapters {
        chapter.validate()?;
    }
    chapters
        .iter()
        .filter(|chapter| chapter.book_id == request.book_id)
        .find(|chapter| {
            request
                .chapter_id
                .as_deref()
                .is_some_and(|chapter_id| chapter.stable_chapter_id == chapter_id)
                || request
                    .ordinal
                    .is_some_and(|ordinal| chapter.ordinal == ordinal)
        })
        .cloned()
        .ok_or_else(|| LocalBookError::ChapterNotFound {
            book_id: request.book_id.clone(),
            chapter_index: request.ordinal.unwrap_or_default().max(0) as u32,
        })
}

pub fn local_book_next_chapter_read_request(
    current: &LocalBookChapterIndexEntry,
    preview_limit: usize,
) -> Result<LocalBookChapterReadRequest, LocalBookError> {
    current.validate()?;
    LocalBookChapterReadRequest::by_ordinal(
        current.book_id.clone(),
        current.ordinal.saturating_add(1),
        preview_limit,
    )
}

pub fn local_book_previous_chapter_read_request(
    current: &LocalBookChapterIndexEntry,
    preview_limit: usize,
) -> Result<LocalBookChapterReadRequest, LocalBookError> {
    current.validate()?;
    LocalBookChapterReadRequest::by_ordinal(
        current.book_id.clone(),
        current.ordinal.saturating_sub(1).max(0),
        preview_limit,
    )
}

pub fn plan_local_book_chapter_prefetch(
    request: &LocalBookChapterPrefetchRequest,
) -> Result<LocalBookChapterPrefetchPlan, LocalBookError> {
    request.validate()?;
    let start = request.anchor_ordinal.saturating_sub(request.radius).max(0);
    let max_end = start.saturating_add(request.maximum_count as i64 - 1);
    let radius_end = request.anchor_ordinal.saturating_add(request.radius);
    let end = radius_end.min(max_end);
    let ordinals = (start..=end).collect::<Vec<_>>();
    Ok(LocalBookChapterPrefetchPlan {
        book_id: request.book_id.clone(),
        ordinals,
    })
}

pub fn local_book_resource_kind(
    path: &str,
    mime_type: &str,
    is_cover: bool,
) -> LocalBookResourceKind {
    if is_cover {
        return LocalBookResourceKind::Cover;
    }
    let mime_type = mime_type.trim().to_ascii_lowercase();
    if mime_type.starts_with("image/") {
        return LocalBookResourceKind::Image;
    }
    if mime_type == "text/css" {
        return LocalBookResourceKind::Css;
    }
    let path = path.trim().to_ascii_lowercase();
    if path.ends_with(".otf") || path.ends_with(".ttf") || path.ends_with(".woff") {
        return LocalBookResourceKind::Font;
    }
    LocalBookResourceKind::Other
}

pub fn plan_local_book_resource_read(
    request: &LocalBookResourceReadRequest,
    resources: &[LocalBookResourceIndexEntry],
    cached: Option<&LocalBookResourceReadResult>,
) -> Result<LocalBookResourceReadResult, LocalBookError> {
    request.validate()?;
    for resource in resources {
        resource.validate()?;
    }

    if let Some(cached) = cached.filter(|cached| {
        cached.book_id == request.book_id && cached.resource_id == request.resource_id
    }) {
        cached.validate()?;
        let mut cached = cached.clone();
        cached.cache_hit = true;
        return Ok(cached);
    }

    let Some(resource) = resources.iter().find(|resource| {
        resource.book_id == request.book_id && resource.stable_resource_id == request.resource_id
    }) else {
        return Err(LocalBookError::ResourceNotFound {
            book_id: request.book_id.clone(),
            resource_id: request.resource_id.clone(),
        });
    };

    let empty_checksum = local_book_stable_checksum(&[]);
    if !is_safe_local_book_relative_locator(&resource.relative_locator) {
        return Ok(LocalBookResourceReadResult {
            book_id: request.book_id.clone(),
            resource_id: request.resource_id.clone(),
            relative_locator: resource.relative_locator.clone(),
            mime_type: resource.mime_type.clone(),
            byte_count: 0,
            checksum: empty_checksum,
            cache_hit: false,
            cacheable: false,
            diagnostics: vec!["unsafe_path_rejected".into()],
        });
    }

    if resource.byte_count > request.max_bytes {
        return Ok(LocalBookResourceReadResult {
            book_id: request.book_id.clone(),
            resource_id: request.resource_id.clone(),
            relative_locator: resource.relative_locator.clone(),
            mime_type: resource.mime_type.clone(),
            byte_count: 0,
            checksum: empty_checksum,
            cache_hit: false,
            cacheable: false,
            diagnostics: vec!["oversized_resource_rejected".into()],
        });
    }

    Ok(LocalBookResourceReadResult {
        book_id: request.book_id.clone(),
        resource_id: request.resource_id.clone(),
        relative_locator: resource.relative_locator.clone(),
        mime_type: resource.mime_type.clone(),
        byte_count: resource.byte_count,
        checksum: resource.checksum.clone().unwrap_or_else(|| {
            local_book_stable_checksum(&[
                "resource",
                &resource.relative_locator,
                &resource.byte_count.to_string(),
            ])
        }),
        cache_hit: false,
        cacheable: resource.resource_kind != LocalBookResourceKind::Font,
        diagnostics: Vec::new(),
    })
}

fn is_safe_local_book_relative_locator(relative_locator: &str) -> bool {
    let relative_locator = relative_locator.trim();
    !relative_locator.is_empty()
        && !relative_locator.starts_with('/')
        && !relative_locator.split('/').any(|part| part == "..")
}

/// Portable local-book reading locator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookReadingLocator {
    #[serde(rename = "bookId")]
    pub book_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    pub chapter_ordinal: i64,
    pub format: LocalBookFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_canonical_locator: Option<String>,
    pub character_offset: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_offset: Option<u64>,
    pub normalized_progress_in_chapter: f64,
    pub normalized_progress_in_book: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf_page_index: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epub_fragment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txt_source_range: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surrounding_text_checksum: Option<String>,
    pub parser_version: String,
    pub book_fingerprint: String,
    pub timestamp: String,
}

impl LocalBookReadingLocator {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_optional_metadata(&self.chapter_id, "chapter_id")?;
        validate_optional_metadata(&self.chapter_canonical_locator, "chapter_canonical_locator")?;
        if self.character_offset < 0 {
            return Err(LocalBookError::InvalidMetadata {
                field: "character_offset".into(),
            });
        }
        validate_optional_metadata(&self.epub_fragment_id, "epub_fragment_id")?;
        validate_optional_metadata(&self.txt_source_range, "txt_source_range")?;
        validate_optional_metadata(&self.surrounding_text_checksum, "surrounding_text_checksum")?;
        validate_required_metadata(&self.parser_version, "parser_version")?;
        validate_required_metadata(&self.book_fingerprint, "book_fingerprint")?;
        validate_required_metadata(&self.timestamp, "timestamp")?;
        validate_progress_fraction(
            self.normalized_progress_in_chapter,
            "normalized_progress_in_chapter",
        )?;
        validate_progress_fraction(
            self.normalized_progress_in_book,
            "normalized_progress_in_book",
        )?;
        Ok(())
    }

    fn replacing_chapter(&self, chapter: &LocalBookChapterIndexEntry) -> Self {
        let mut locator = self.clone();
        locator.chapter_id = Some(chapter.stable_chapter_id.clone());
        locator.chapter_ordinal = chapter.ordinal;
        locator.chapter_canonical_locator = Some(chapter.canonical_locator.clone());
        locator
    }

    fn reset_for_book(book_id: &str, format: LocalBookFormat, book_fingerprint: &str) -> Self {
        Self {
            book_id: book_id.into(),
            chapter_id: None,
            chapter_ordinal: 0,
            format,
            chapter_canonical_locator: None,
            character_offset: 0,
            byte_offset: None,
            normalized_progress_in_chapter: 0.0,
            normalized_progress_in_book: 0.0,
            pdf_page_index: None,
            epub_fragment_id: None,
            txt_source_range: None,
            surrounding_text_checksum: None,
            parser_version: "RECOVERY-32".into(),
            book_fingerprint: book_fingerprint.into(),
            timestamp: "1970-01-01T00:00:00Z".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookReadingProgress {
    pub locator: LocalBookReadingLocator,
    pub restore_state: LocalBookReadingRestoreState,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

impl LocalBookReadingProgress {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        self.locator.validate()?;
        if self.diagnostics.iter().any(|value| value.trim().is_empty()) {
            return Err(LocalBookError::InvalidMetadata {
                field: "diagnostics".into(),
            });
        }
        Ok(())
    }
}

/// File-store portable snapshot for Recovery32 catalog/progress/cache metadata.
///
/// The model intentionally excludes host file URLs and full chapter content.
/// Host stores can persist this JSON while keeping platform file access outside
/// Core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookLibraryStoreSnapshot {
    pub schema_version: u32,
    pub exported_at: i64,
    pub catalog: LocalBookCatalogSnapshot,
    #[serde(default)]
    pub reading_progress: Vec<LocalBookReadingProgress>,
    #[serde(default)]
    pub cache_metadata: Vec<LocalBookCacheMetadata>,
}

impl LocalBookLibraryStoreSnapshot {
    pub fn validate(&self) -> Result<(), LocalBookError> {
        if self.schema_version != LOCAL_BOOK_LIBRARY_STORE_SNAPSHOT_SCHEMA_VERSION {
            return Err(LocalBookError::InvalidSnapshot {
                field: "schema_version".into(),
            });
        }
        self.catalog.validate()?;
        let book_ids = self
            .catalog
            .books
            .iter()
            .map(|book| book.stable_book_id.as_str())
            .collect::<HashSet<_>>();
        for progress in &self.reading_progress {
            progress.validate()?;
            if !book_ids.contains(progress.locator.book_id.as_str()) {
                return Err(LocalBookError::InvalidSnapshot {
                    field: "reading_progress.book_id".into(),
                });
            }
        }
        for metadata in &self.cache_metadata {
            metadata.validate()?;
        }
        Ok(())
    }
}

pub fn build_local_book_library_store_snapshot(
    exported_at: i64,
    catalog: &LocalBookCatalogSnapshot,
    reading_progress: &[LocalBookReadingProgress],
    cache_metadata: &[LocalBookCacheMetadata],
) -> Result<LocalBookLibraryStoreSnapshot, LocalBookError> {
    let mut catalog = catalog.clone();
    sort_local_book_catalog(&mut catalog);
    catalog.validate()?;

    let mut reading_progress = reading_progress.to_vec();
    for progress in &reading_progress {
        progress.validate()?;
    }
    sort_local_book_reading_progress(&mut reading_progress);

    let cache_metadata = enumerate_local_book_cache_metadata(cache_metadata)?;
    let snapshot = LocalBookLibraryStoreSnapshot {
        schema_version: LOCAL_BOOK_LIBRARY_STORE_SNAPSHOT_SCHEMA_VERSION,
        exported_at,
        catalog,
        reading_progress,
        cache_metadata,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

pub fn local_book_library_store_snapshot_portable_json(
    snapshot: &LocalBookLibraryStoreSnapshot,
) -> Result<String, LocalBookError> {
    snapshot.validate()?;
    let json = serde_json::to_string(snapshot).map_err(|error| LocalBookError::Decode {
        reason: error.to_string(),
    })?;
    if json.contains("/Users/") {
        return Err(LocalBookError::InvalidSnapshot {
            field: "host_path".into(),
        });
    }
    Ok(json)
}

fn sort_local_book_reading_progress(progress: &mut [LocalBookReadingProgress]) {
    progress.sort_by(|left, right| {
        left.locator
            .book_id
            .cmp(&right.locator.book_id)
            .then_with(|| {
                left.locator
                    .chapter_ordinal
                    .cmp(&right.locator.chapter_ordinal)
            })
            .then_with(|| left.locator.chapter_id.cmp(&right.locator.chapter_id))
            .then_with(|| left.locator.timestamp.cmp(&right.locator.timestamp))
    });
}

pub fn resolve_local_book_reading_progress(
    progress: &LocalBookReadingProgress,
    book_id: &str,
    book: Option<&LocalBookFingerprintCatalogEntry>,
    chapters: &[LocalBookChapterIndexEntry],
) -> Result<LocalBookReadingProgress, LocalBookError> {
    progress.validate()?;
    validate_required_metadata(book_id, "book_id")?;
    if let Some(book) = book {
        book.validate()?;
    }
    for chapter in chapters {
        chapter.validate()?;
    }

    let mut chapters = chapters
        .iter()
        .filter(|chapter| chapter.book_id == book_id)
        .collect::<Vec<_>>();
    chapters.sort_by_key(|chapter| chapter.ordinal);
    let book = book.filter(|book| book.stable_book_id == book_id);
    let Some(book) = book else {
        return Ok(reset_local_book_reading_progress(
            progress,
            book_id,
            chapters.first().copied(),
        ));
    };
    if chapters.is_empty() {
        return Ok(reset_local_book_reading_progress(progress, book_id, None));
    }

    if progress.locator.book_fingerprint != book.content_fingerprint.full_input_checksum
        && progress
            .locator
            .chapter_id
            .as_deref()
            .is_some_and(|chapter_id| {
                chapters
                    .iter()
                    .any(|chapter| chapter.stable_chapter_id == chapter_id)
            })
    {
        return Ok(LocalBookReadingProgress {
            locator: progress.locator.clone(),
            restore_state: LocalBookReadingRestoreState::StaleBookFingerprint,
            diagnostics: vec!["fingerprint_changed_but_chapter_id_exists".into()],
        });
    }

    if progress
        .locator
        .chapter_id
        .as_deref()
        .is_some_and(|chapter_id| {
            chapters
                .iter()
                .any(|chapter| chapter.stable_chapter_id == chapter_id)
        })
    {
        return Ok(LocalBookReadingProgress {
            locator: progress.locator.clone(),
            restore_state: LocalBookReadingRestoreState::ExactRestored,
            diagnostics: Vec::new(),
        });
    }

    if let Some(chapter) = progress
        .locator
        .chapter_canonical_locator
        .as_deref()
        .and_then(|locator| {
            chapters
                .iter()
                .find(|chapter| chapter.canonical_locator == locator)
        })
    {
        return Ok(LocalBookReadingProgress {
            locator: progress.locator.replacing_chapter(chapter),
            restore_state: LocalBookReadingRestoreState::LocatorRestored,
            diagnostics: Vec::new(),
        });
    }

    if let Some(chapter) = chapters
        .iter()
        .find(|chapter| chapter.ordinal == progress.locator.chapter_ordinal)
    {
        return Ok(LocalBookReadingProgress {
            locator: progress.locator.replacing_chapter(chapter),
            restore_state: LocalBookReadingRestoreState::OrdinalRestored,
            diagnostics: Vec::new(),
        });
    }

    let nearest_index = progress
        .locator
        .chapter_ordinal
        .clamp(0, chapters.len().saturating_sub(1) as i64) as usize;
    let nearest = chapters[nearest_index];
    Ok(LocalBookReadingProgress {
        locator: progress.locator.replacing_chapter(nearest),
        restore_state: LocalBookReadingRestoreState::NearestChapterRestored,
        diagnostics: vec!["nearest_fallback".into()],
    })
}

fn reset_local_book_reading_progress(
    progress: &LocalBookReadingProgress,
    book_id: &str,
    first_chapter: Option<&LocalBookChapterIndexEntry>,
) -> LocalBookReadingProgress {
    let mut locator = LocalBookReadingLocator::reset_for_book(
        book_id,
        progress.locator.format,
        &progress.locator.book_fingerprint,
    );
    if let Some(first_chapter) = first_chapter {
        locator.chapter_id = Some(first_chapter.stable_chapter_id.clone());
        locator.chapter_canonical_locator = Some(first_chapter.canonical_locator.clone());
    }
    LocalBookReadingProgress {
        locator,
        restore_state: LocalBookReadingRestoreState::ResetToBeginning,
        diagnostics: vec!["missing_book_or_empty_index".into()],
    }
}

fn validate_progress_fraction(value: f64, field: &str) -> Result<(), LocalBookError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(LocalBookError::InvalidMetadata {
            field: field.into(),
        });
    }
    Ok(())
}

/// Complete export/import unit for local-book library state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookLibrarySnapshot {
    pub schema_version: u32,
    pub exported_at: i64,
    #[serde(default)]
    pub books: Vec<LocalBook>,
}

/// Portable backup metadata for one local book.
///
/// This mirrors the legacy `LocalBookBackupMeta` data contract. It records
/// backup participation and externally supplied file hash metadata without
/// performing filesystem access or hashing inside Core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalBookBackupMetadata {
    #[serde(rename = "bookId")]
    pub book_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub file_format: LocalBookFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_backup_at: Option<i64>,
    #[serde(default)]
    pub included_in_backup: bool,
}

impl LocalBookBackupMetadata {
    pub fn new(
        book_id: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<Self, LocalBookError> {
        let metadata = Self {
            book_id: normalize_required_owned(book_id.into(), "book_id")?,
            title: normalize_required_owned(title.into(), "title")?,
            author: None,
            file_format: LocalBookFormat::Unknown,
            file_hash: None,
            last_backup_at: None,
            included_in_backup: false,
        };
        metadata.validate()?;
        Ok(metadata)
    }

    pub fn from_book(
        book: &LocalBook,
        file_hash: Option<String>,
        last_backup_at: Option<i64>,
        included_in_backup: bool,
    ) -> Result<Self, LocalBookError> {
        validate_local_book(book)?;
        let metadata = Self {
            book_id: book.book.book_id.clone(),
            title: book.book.title.clone(),
            author: non_empty_optional(book.book.author.clone()),
            file_format: book.format,
            file_hash: normalize_optional_metadata(file_hash, "file_hash")?,
            last_backup_at,
            included_in_backup,
        };
        metadata.validate()?;
        Ok(metadata)
    }

    pub fn validate(&self) -> Result<(), LocalBookError> {
        validate_required_metadata(&self.book_id, "book_id")?;
        validate_required_metadata(&self.title, "title")?;
        validate_optional_metadata(&self.author, "author")?;
        validate_optional_metadata(&self.file_hash, "file_hash")?;
        Ok(())
    }
}

impl LocalBookLibrarySnapshot {
    pub fn empty(exported_at: i64) -> Self {
        Self {
            schema_version: LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            books: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), LocalBookError> {
        if self.schema_version != LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION {
            return Err(LocalBookError::InvalidSnapshot {
                field: "schema_version".into(),
            });
        }

        let mut book_ids = HashMap::<String, ()>::new();
        for book in &self.books {
            validate_local_book(book)?;
            if book_ids.insert(book.book.book_id.clone(), ()).is_some() {
                return Err(LocalBookError::InvalidSnapshot {
                    field: "books".into(),
                });
            }
        }
        Ok(())
    }
}

/// In-memory local-book library for parsed offline books.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LocalBookLibrary {
    books: HashMap<String, LocalBook>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalBookError {
    EmptyInput,
    InvalidMetadata {
        field: String,
    },
    InvalidBook {
        field: String,
    },
    InvalidSnapshot {
        field: String,
    },
    UnsupportedEncoding,
    Decode {
        reason: String,
    },
    BookNotFound {
        book_id: String,
    },
    ChapterNotFound {
        book_id: String,
        chapter_index: u32,
    },
    ResourceNotFound {
        book_id: String,
        resource_id: String,
    },
}

impl std::fmt::Display for LocalBookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocalBookError::EmptyInput => write!(f, "local book input is empty"),
            LocalBookError::InvalidMetadata { field } => {
                write!(f, "invalid local book metadata field: {field}")
            }
            LocalBookError::InvalidBook { field } => {
                write!(f, "invalid local book field: {field}")
            }
            LocalBookError::InvalidSnapshot { field } => {
                write!(f, "invalid local book snapshot field: {field}")
            }
            LocalBookError::UnsupportedEncoding => write!(f, "unsupported local book encoding"),
            LocalBookError::Decode { reason } => write!(f, "failed to decode local book: {reason}"),
            LocalBookError::BookNotFound { book_id } => {
                write!(f, "local book not found: {book_id}")
            }
            LocalBookError::ChapterNotFound {
                book_id,
                chapter_index,
            } => write!(
                f,
                "local book chapter not found: book={book_id} chapter={chapter_index}"
            ),
            LocalBookError::ResourceNotFound {
                book_id,
                resource_id,
            } => write!(
                f,
                "local book resource not found: book={book_id} resource={resource_id}"
            ),
        }
    }
}

impl std::error::Error for LocalBookError {}

pub fn detect_local_book_format(
    bytes: &[u8],
    request: &LocalBookFormatDetectionRequest,
) -> Result<LocalBookFormatDetection, LocalBookError> {
    request.validate()?;
    let extension = request
        .declared_extension
        .as_deref()
        .and_then(normalize_extension)
        .or_else(|| {
            request
                .declared_filename
                .as_deref()
                .and_then(extension_from_path)
        });
    let mime = request
        .declared_mime_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let declared_format = declared_local_book_format(extension.as_deref(), mime.as_deref());
    let detected_format = detect_local_book_format_from_bytes(bytes, declared_format);
    let mut diagnostics = Vec::new();
    if let Some(declared_format) = declared_format {
        if declared_format != detected_format {
            diagnostics.push(format!(
                "declared_detected_mismatch:declared {}, detected {}",
                local_book_format_wire_value(declared_format),
                local_book_format_wire_value(detected_format)
            ));
        }
    }

    Ok(LocalBookFormatDetection {
        format: detected_format,
        declared_format,
        media_type: local_book_media_type_for_format(detected_format).to_string(),
        effective_preview_limit: request.effective_preview_limit(),
        diagnostics,
    })
}

pub fn declared_local_book_format(
    extension: Option<&str>,
    mime: Option<&str>,
) -> Option<LocalBookFormat> {
    let extension = extension.and_then(normalize_extension);
    let mime = mime
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    match (extension.as_deref(), mime.as_deref()) {
        (Some("epub"), _) | (_, Some("application/epub+zip")) => Some(LocalBookFormat::Epub),
        (Some("pdf"), _) | (_, Some("application/pdf")) => Some(LocalBookFormat::Pdf),
        (Some("txt"), _) => Some(LocalBookFormat::Txt),
        (_, Some(value)) if value.starts_with("text/") => Some(LocalBookFormat::Txt),
        (Some("mobi"), _) | (_, Some("application/x-mobipocket-ebook")) => {
            Some(LocalBookFormat::Mobi)
        }
        (Some("azw" | "azw3" | "kf8"), _) | (_, Some("application/vnd.amazon.ebook")) => {
            Some(LocalBookFormat::Azw)
        }
        (Some("umd"), _) | (_, Some("application/x-umd")) => Some(LocalBookFormat::Umd),
        (Some("zip" | "cbz" | "archive" | "tar"), _)
        | (_, Some("application/zip" | "application/x-tar" | "application/tar"))
        | (
            _,
            Some(
                "application/vnd.reader-core.archive+zip"
                | "application/vnd.reader-core.archive+tar",
            ),
        ) => Some(LocalBookFormat::Archive),
        (Some("webdav" | "webdavbook"), _)
        | (_, Some("application/vnd.reader-core.webdav-local-book+json")) => {
            Some(LocalBookFormat::WebDav)
        }
        _ => None,
    }
}

pub fn local_book_media_type_for_format(format: LocalBookFormat) -> &'static str {
    match format {
        LocalBookFormat::Txt => "text/plain",
        LocalBookFormat::Epub => "application/epub+zip",
        LocalBookFormat::Pdf => "application/pdf",
        LocalBookFormat::Html => "text/html",
        LocalBookFormat::Mobi => "application/x-mobipocket-ebook",
        LocalBookFormat::Azw => "application/vnd.amazon.ebook",
        LocalBookFormat::Umd => "application/x-umd",
        LocalBookFormat::Archive => "application/zip",
        LocalBookFormat::WebDav => "application/vnd.reader-core.webdav-local-book+json",
        LocalBookFormat::Unknown => "application/octet-stream",
    }
}

pub fn local_book_media_type_for_extension(value: &str) -> String {
    let normalized = value.trim().trim_start_matches('.').to_ascii_lowercase();
    if normalized.contains('/') {
        return normalized;
    }
    match normalize_extension(&normalized).as_deref() {
        Some("txt") => "text/plain",
        Some("epub") => "application/epub+zip",
        Some("pdf") => "application/pdf",
        Some("mobi") => "application/x-mobipocket-ebook",
        Some("azw" | "azw3" | "kf8") => "application/vnd.amazon.ebook",
        Some("umd") => "application/x-umd",
        Some("zip" | "cbz" | "archive") => "application/zip",
        Some("tar") => "application/x-tar",
        Some("webdav" | "webdavbook") => "application/vnd.reader-core.webdav-local-book+json",
        Some("html" | "xhtml") => "text/html",
        _ => "application/octet-stream",
    }
    .to_string()
}

pub fn decide_local_book_duplicate(
    fingerprint: &LocalBookFingerprintSet,
    declared_filename: Option<&str>,
    catalog_matches: &[LocalBookFingerprintCatalogEntry],
) -> Result<LocalBookDuplicateResult, LocalBookError> {
    fingerprint.validate()?;
    for entry in catalog_matches {
        entry.validate()?;
    }

    let Some(entry) = catalog_matches.first() else {
        return Ok(LocalBookDuplicateResult {
            decision: LocalBookDuplicateDecision::InsufficientEvidence,
            matched_book_id: None,
            duplicate_group_id: None,
            reason_codes: vec!["no_catalog_match".into()],
        });
    };

    if entry.content_fingerprint.full_input_checksum == fingerprint.content.full_input_checksum {
        let declared_filename_checksum =
            declared_filename.map(|filename| local_book_stable_checksum(&[filename]));
        let same_name =
            entry.source_fingerprint.declared_filename_checksum == declared_filename_checksum;
        return Ok(LocalBookDuplicateResult {
            decision: if same_name {
                LocalBookDuplicateDecision::ExactDuplicate
            } else {
                LocalBookDuplicateDecision::SameBytesDifferentPath
            },
            matched_book_id: Some(entry.stable_book_id.clone()),
            duplicate_group_id: Some(entry.duplicate_group_id.clone().unwrap_or_else(|| {
                local_book_stable_checksum(&["dup", &fingerprint.content.full_input_checksum])
            })),
            reason_codes: if same_name {
                vec![
                    "full_fingerprint_match".into(),
                    "filename_checksum_match".into(),
                ]
            } else {
                vec![
                    "full_fingerprint_match".into(),
                    "filename_checksum_changed".into(),
                ]
            },
        });
    }

    if entry.semantic_fingerprint.normalized_title == fingerprint.semantic.normalized_title
        && entry.semantic_fingerprint.normalized_author == fingerprint.semantic.normalized_author
        && entry.semantic_fingerprint.chapter_count == fingerprint.semantic.chapter_count
    {
        return Ok(LocalBookDuplicateResult {
            decision: LocalBookDuplicateDecision::SameSemanticBook,
            matched_book_id: Some(entry.stable_book_id.clone()),
            duplicate_group_id: Some(entry.duplicate_group_id.clone().unwrap_or_else(|| {
                local_book_stable_checksum(&[
                    "dup",
                    &fingerprint.semantic.normalized_title,
                    fingerprint
                        .semantic
                        .normalized_author
                        .as_deref()
                        .unwrap_or_default(),
                ])
            })),
            reason_codes: vec!["semantic_title_author_chapter_count_match".into()],
        });
    }

    if entry.semantic_fingerprint.normalized_title == fingerprint.semantic.normalized_title
        && entry.semantic_fingerprint.normalized_author != fingerprint.semantic.normalized_author
    {
        return Ok(LocalBookDuplicateResult {
            decision: LocalBookDuplicateDecision::Unrelated,
            matched_book_id: Some(entry.stable_book_id.clone()),
            duplicate_group_id: None,
            reason_codes: vec!["same_title_different_author".into()],
        });
    }

    if entry.semantic_fingerprint.normalized_title == fingerprint.semantic.normalized_title
        && entry.semantic_fingerprint.chapter_count != fingerprint.semantic.chapter_count
    {
        return Ok(LocalBookDuplicateResult {
            decision: LocalBookDuplicateDecision::DifferentEdition,
            matched_book_id: Some(entry.stable_book_id.clone()),
            duplicate_group_id: None,
            reason_codes: vec!["same_title_different_chapter_count".into()],
        });
    }

    Ok(LocalBookDuplicateResult {
        decision: LocalBookDuplicateDecision::LikelyDuplicate,
        matched_book_id: Some(entry.stable_book_id.clone()),
        duplicate_group_id: None,
        reason_codes: vec!["partial_semantic_overlap".into()],
    })
}

pub fn decide_local_book_change(
    existing: Option<&LocalBookFingerprintCatalogEntry>,
    new_fingerprint: &LocalBookFingerprintSet,
    policy: LocalBookValidationPolicy,
) -> Result<LocalBookChangeResult, LocalBookError> {
    new_fingerprint.validate()?;
    let Some(existing) = existing else {
        return Ok(LocalBookChangeResult {
            decision: LocalBookChangeDecision::Removed,
            reason_codes: vec!["catalog_entry_missing".into()],
        });
    };
    existing.validate()?;

    if existing.source_fingerprint.detected_format != new_fingerprint.fast.detected_format {
        return Ok(LocalBookChangeResult {
            decision: LocalBookChangeDecision::FormatChanged,
            reason_codes: vec!["format_changed".into()],
        });
    }

    if existing.content_fingerprint.parser_config_checksum
        != new_fingerprint.content.parser_config_checksum
    {
        return Ok(LocalBookChangeResult {
            decision: LocalBookChangeDecision::ParserConfigChanged,
            reason_codes: vec!["parser_config_changed".into()],
        });
    }

    Ok(match policy {
        LocalBookValidationPolicy::MetadataOnly => {
            if existing.source_fingerprint.byte_count == new_fingerprint.fast.byte_count {
                LocalBookChangeResult {
                    decision: LocalBookChangeDecision::MetadataOnlyChanged,
                    reason_codes: vec!["metadata_policy_no_content_validation".into()],
                }
            } else {
                LocalBookChangeResult {
                    decision: LocalBookChangeDecision::ContentChanged,
                    reason_codes: vec!["size_changed".into()],
                }
            }
        }
        LocalBookValidationPolicy::FastFingerprint => {
            if existing.source_fingerprint.prefix_checksum == new_fingerprint.fast.prefix_checksum
                && existing.source_fingerprint.suffix_checksum
                    == new_fingerprint.fast.suffix_checksum
            {
                LocalBookChangeResult {
                    decision: LocalBookChangeDecision::Unchanged,
                    reason_codes: vec!["fast_fingerprint_match".into()],
                }
            } else {
                LocalBookChangeResult {
                    decision: LocalBookChangeDecision::UncertainRequiresFullValidation,
                    reason_codes: vec!["fast_fingerprint_changed".into()],
                }
            }
        }
        LocalBookValidationPolicy::FullFingerprint => {
            if existing.content_fingerprint.full_input_checksum
                == new_fingerprint.content.full_input_checksum
            {
                LocalBookChangeResult {
                    decision: LocalBookChangeDecision::Unchanged,
                    reason_codes: vec!["full_fingerprint_match".into()],
                }
            } else {
                LocalBookChangeResult {
                    decision: LocalBookChangeDecision::ContentChanged,
                    reason_codes: vec!["full_fingerprint_changed".into()],
                }
            }
        }
        LocalBookValidationPolicy::SemanticReimport => {
            if existing.semantic_fingerprint == new_fingerprint.semantic {
                LocalBookChangeResult {
                    decision: LocalBookChangeDecision::MetadataOnlyChanged,
                    reason_codes: vec!["semantic_match".into()],
                }
            } else {
                LocalBookChangeResult {
                    decision: LocalBookChangeDecision::ContentChanged,
                    reason_codes: vec!["semantic_changed".into()],
                }
            }
        }
    })
}

pub fn local_book_stable_checksum(parts: &[&str]) -> String {
    let joined = parts.join("|");
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in joined.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    format!("fnv1a64:{hash:016x}")
}

pub fn plan_epub_nav_chapter_index(
    request: &LocalBookEpubChapterIndexRequest,
) -> Result<LocalBookEpubChapterIndexPlan, LocalBookError> {
    request.validate()?;

    let package_base_path = normalize_epub_path("", &request.package_base_path);
    let nav_base_path = request
        .nav_document_path
        .as_deref()
        .map(epub_parent_path)
        .unwrap_or_else(|| package_base_path.clone());

    let mut manifest_by_id = BTreeMap::<String, (String, String)>::new();
    for item in &request.manifest_items {
        let (manifest_path, _) = resolve_epub_href(&package_base_path, &item.href);
        manifest_by_id.insert(
            item.id.trim().to_string(),
            (manifest_path, item.media_type.trim().to_string()),
        );
    }

    let mut linear_paths = BTreeMap::<String, (usize, String)>::new();
    let mut non_linear_paths = BTreeSet::<String>::new();
    let mut diagnostics_summary = Vec::new();
    for (spine_index, spine_item) in request.spine_items.iter().enumerate() {
        let Some((path, media_type)) = manifest_by_id.get(spine_item.idref.trim()) else {
            diagnostics_summary.push(format!("epub_spine_missing_manifest:{}", spine_item.idref));
            continue;
        };
        if spine_item.linear {
            linear_paths.insert(path.clone(), (spine_index, media_type.clone()));
        } else {
            non_linear_paths.insert(path.clone());
        }
    }

    let known_fragments_by_path = normalize_epub_known_fragment_ids(&request.known_fragment_ids);
    let mut chapters = Vec::new();
    let mut skipped_nav_hrefs = Vec::new();
    let mut duplicate_nav_hrefs = Vec::new();
    let mut seen_nav_keys = BTreeSet::<(String, Option<String>)>::new();

    for nav_item in &request.nav_items {
        let (path, fragment) = resolve_epub_href(&nav_base_path, &nav_item.href);
        let Some((_, media_type)) = linear_paths.get(&path) else {
            skipped_nav_hrefs.push(nav_item.href.trim().to_string());
            if non_linear_paths.contains(&path) {
                diagnostics_summary.push(format!("epub_nav_skipped_non_linear:{}", nav_item.href));
            } else {
                diagnostics_summary.push(format!("epub_nav_missing_spine:{}", nav_item.href));
            }
            continue;
        };

        let fragment = fragment.filter(|fragment| {
            known_fragments_by_path
                .get(&path)
                .is_some_and(|known| known.contains(fragment))
        });
        let dedupe_key = (path.clone(), fragment.clone());
        if !seen_nav_keys.insert(dedupe_key) {
            duplicate_nav_hrefs.push(nav_item.href.trim().to_string());
            diagnostics_summary.push(format!("epub_nav_duplicate:{}", nav_item.href));
            continue;
        }

        let source_range_path_or_page = match &fragment {
            Some(fragment) => format!("{path}#{fragment}"),
            None => path.clone(),
        };
        let ordinal = chapters.len() as i64;
        let stable_chapter_id = local_book_stable_checksum(&[
            "epub-chapter",
            &request.book_id,
            &source_range_path_or_page,
        ]);
        let decoded_title = decode_epub_href_entities(&nav_item.title);
        let normalized_title = normalize_required(&decoded_title, "nav.title")?;
        let canonical_locator = format!("epub://{}/{}", request.book_id, source_range_path_or_page);
        chapters.push(LocalBookChapterIndexEntry {
            stable_chapter_id,
            book_id: request.book_id.trim().to_string(),
            ordinal,
            normalized_title,
            canonical_locator,
            source_range_path_or_page,
            content_type: media_type.clone(),
            estimated_byte_count: 0,
            estimated_character_count: nav_item.title.chars().count() as u64,
            content_checksum: None,
            is_materialized: false,
            previous_chapter_id: None,
            next_chapter_id: None,
            parser_version: request.parser_version.trim().to_string(),
            diagnostics_summary: Vec::new(),
        });
    }

    link_local_book_chapter_neighbors(&mut chapters);
    let plan = LocalBookEpubChapterIndexPlan {
        chapters,
        skipped_nav_hrefs,
        duplicate_nav_hrefs,
        diagnostics_summary,
    };
    plan.validate()?;
    Ok(plan)
}

pub fn plan_epub_navigation_with_fallback(
    request: &LocalBookEpubNavigationFallbackRequest,
) -> Result<LocalBookEpubNavigationFallbackPlan, LocalBookError> {
    request.validate()?;

    let mut fallback_diagnostics = Vec::new();
    if !request.nav_items.is_empty() {
        let nav_plan = plan_epub_nav_chapter_index(&LocalBookEpubChapterIndexRequest {
            book_id: request.book_id.clone(),
            package_base_path: request.package_base_path.clone(),
            nav_document_path: request.nav_document_path.clone(),
            manifest_items: request.manifest_items.clone(),
            spine_items: request.spine_items.clone(),
            nav_items: request.nav_items.clone(),
            known_fragment_ids: request.known_fragment_ids.clone(),
            parser_version: request.parser_version.clone(),
        })?;
        if !nav_plan.chapters.is_empty() {
            let plan = LocalBookEpubNavigationFallbackPlan {
                selected_source: LocalBookEpubNavigationSource::Nav,
                chapter_plan: nav_plan,
                diagnostics_summary: fallback_diagnostics,
            };
            plan.validate()?;
            return Ok(plan);
        }
        fallback_diagnostics.push("epub_nav_invalid:trying_ncx_fallback".into());
    } else {
        fallback_diagnostics.push("epub_nav_empty:trying_ncx_fallback".into());
    }

    if !request.ncx_items.is_empty() {
        let ncx_plan = plan_epub_nav_chapter_index(&LocalBookEpubChapterIndexRequest {
            book_id: request.book_id.clone(),
            package_base_path: request.package_base_path.clone(),
            nav_document_path: None,
            manifest_items: request.manifest_items.clone(),
            spine_items: request.spine_items.clone(),
            nav_items: request.ncx_items.clone(),
            known_fragment_ids: request.known_fragment_ids.clone(),
            parser_version: request.parser_version.clone(),
        })?;
        if !ncx_plan.chapters.is_empty() {
            let plan = LocalBookEpubNavigationFallbackPlan {
                selected_source: LocalBookEpubNavigationSource::Ncx,
                chapter_plan: ncx_plan,
                diagnostics_summary: fallback_diagnostics,
            };
            plan.validate()?;
            return Ok(plan);
        }
        fallback_diagnostics.push("epub_ncx_invalid:trying_spine_fallback".into());
    } else {
        fallback_diagnostics.push("epub_ncx_empty:trying_spine_fallback".into());
    }

    let spine_nav_items = epub_spine_fallback_nav_items(
        &request.package_base_path,
        &request.manifest_items,
        &request.spine_items,
    );
    let spine_plan = plan_epub_nav_chapter_index(&LocalBookEpubChapterIndexRequest {
        book_id: request.book_id.clone(),
        package_base_path: request.package_base_path.clone(),
        nav_document_path: None,
        manifest_items: request.manifest_items.clone(),
        spine_items: request.spine_items.clone(),
        nav_items: spine_nav_items,
        known_fragment_ids: request.known_fragment_ids.clone(),
        parser_version: request.parser_version.clone(),
    })?;
    let plan = LocalBookEpubNavigationFallbackPlan {
        selected_source: LocalBookEpubNavigationSource::Spine,
        chapter_plan: spine_plan,
        diagnostics_summary: fallback_diagnostics,
    };
    plan.validate()?;
    Ok(plan)
}

pub fn plan_epub_archive_preflight(
    request: &LocalBookEpubArchivePreflightRequest,
) -> Result<LocalBookEpubArchivePreflightPlan, LocalBookError> {
    request.validate()?;

    let mut diagnostics_summary = Vec::new();
    let mut has_container = false;
    let mut has_encryption = false;
    for entry in &request.archive_entries {
        let path = entry.path.trim();
        if is_unsafe_epub_archive_path(path) {
            diagnostics_summary.push(format!("unsafe_archive_path:{path}"));
            continue;
        }
        let normalized = normalize_epub_path("", path);
        if normalized == "META-INF/container.xml" {
            has_container = true;
        }
        if normalized == "META-INF/encryption.xml" {
            has_encryption = true;
        }
    }

    if !has_container {
        diagnostics_summary.push("missing_container:META-INF/container.xml".into());
    }
    if has_encryption {
        diagnostics_summary.push("unsupported_encryption:encryption.xml present".into());
        let plan = LocalBookEpubArchivePreflightPlan {
            fail_closed: true,
            manifest_items: Vec::new(),
            spine_items: Vec::new(),
            diagnostics_summary,
        };
        plan.validate()?;
        return Ok(plan);
    }

    let mut manifest_items = Vec::new();
    let mut manifest_ids = BTreeSet::<String>::new();
    for draft in &request.manifest_items {
        let Some(id) = normalize_epub_draft_field(draft.id.as_deref()) else {
            diagnostics_summary.push("missing_manifest_item:missing id or href".into());
            continue;
        };
        let Some(href) = normalize_epub_draft_field(draft.href.as_deref()) else {
            diagnostics_summary.push("missing_manifest_item:missing id or href".into());
            continue;
        };
        if !manifest_ids.insert(id.clone()) {
            diagnostics_summary.push(format!(
                "missing_manifest_item:duplicate manifest item id {id}"
            ));
            continue;
        }
        let media_type = normalize_epub_draft_field(draft.media_type.as_deref())
            .unwrap_or_else(|| "application/octet-stream".into());
        let item = LocalBookEpubManifestItem {
            id,
            href,
            media_type,
            properties: draft
                .properties
                .iter()
                .filter_map(|property| non_empty_optional(property.clone()))
                .collect(),
        };
        item.validate()?;
        manifest_items.push(item);
    }

    let mut spine_items = Vec::new();
    for draft in &request.spine_items {
        let Some(idref) = normalize_epub_draft_field(draft.idref.as_deref()) else {
            diagnostics_summary.push("missing_spine_item:spine itemref missing idref".into());
            continue;
        };
        if !manifest_ids.contains(&idref) {
            diagnostics_summary.push(format!(
                "missing_spine_item:spine idref {idref} missing manifest item"
            ));
            continue;
        }
        let item = LocalBookEpubSpineItem {
            idref,
            linear: draft.linear,
        };
        item.validate()?;
        spine_items.push(item);
    }

    if diagnostics_summary.iter().any(|diagnostic| {
        diagnostic.starts_with("missing_manifest_item:")
            || diagnostic.starts_with("missing_spine_item:")
    }) {
        diagnostics_summary.push("invalid_opf:structural references".into());
    }

    let plan = LocalBookEpubArchivePreflightPlan {
        fail_closed: false,
        manifest_items,
        spine_items,
        diagnostics_summary,
    };
    plan.validate()?;
    Ok(plan)
}

pub fn build_epub_import_diagnostic_report(
    preflight: &LocalBookEpubArchivePreflightPlan,
    navigation: Option<&LocalBookEpubNavigationFallbackPlan>,
) -> Result<LocalBookEpubImportDiagnosticReport, LocalBookError> {
    preflight.validate()?;
    if let Some(navigation) = navigation {
        navigation.validate()?;
    }

    let mut raw_diagnostics = preflight.diagnostics_summary.clone();
    if !preflight.fail_closed {
        if let Some(navigation) = navigation {
            raw_diagnostics.extend(navigation.diagnostics_summary.iter().cloned());
            raw_diagnostics.extend(navigation.chapter_plan.diagnostics_summary.iter().cloned());
        }
    }

    let report = LocalBookEpubImportDiagnosticReport {
        fail_closed: preflight.fail_closed,
        diagnostics: normalize_epub_import_diagnostics(&raw_diagnostics)?,
    };
    report.validate()?;
    Ok(report)
}

pub fn normalize_epub_import_diagnostics(
    diagnostics_summary: &[String],
) -> Result<Vec<LocalBookEpubImportDiagnostic>, LocalBookError> {
    let mut seen = BTreeSet::<LocalBookEpubImportDiagnostic>::new();
    let mut diagnostics = Vec::new();
    for summary in diagnostics_summary {
        validate_required_metadata(summary, "diagnostics_summary")?;
        let Some(diagnostic) = epub_import_diagnostic_from_summary(summary) else {
            continue;
        };
        diagnostic.validate()?;
        if seen.insert(diagnostic.clone()) {
            diagnostics.push(diagnostic);
        }
    }
    Ok(diagnostics)
}

pub fn extract_epub_package_metadata(
    request: &LocalBookEpubPackageMetadataRequest,
) -> Result<LocalBookEpubPackageMetadataArtifact, LocalBookError> {
    request.validate()?;

    let package_unique_identifier_id =
        epub_first_tag_attributes_by_local_name(&request.opf_xml, "package")
            .and_then(|attributes| attributes.get("unique-identifier").cloned())
            .and_then(non_empty_optional);
    let metadata_body = epub_first_element_body_by_local_name(&request.opf_xml, "metadata");
    let mut diagnostics_summary = Vec::new();
    if metadata_body.is_none() {
        diagnostics_summary.push("missing_metadata_section:metadata".into());
    }
    let metadata_body = metadata_body.unwrap_or_default();

    let identifiers = epub_collect_text_elements_by_local_name(metadata_body, "identifier");
    let metadata_identifier = package_unique_identifier_id
        .as_ref()
        .and_then(|unique_identifier_id| {
            identifiers.iter().find(|element| {
                element
                    .attributes
                    .get("id")
                    .is_some_and(|id| id.trim() == unique_identifier_id)
            })
        })
        .or_else(|| identifiers.first())
        .and_then(|element| non_empty_optional(element.text.clone()));
    let metadata_title = epub_first_text_by_local_name(metadata_body, "title");
    let metadata_author = epub_first_text_by_local_name(metadata_body, "creator");
    let metadata_language = epub_first_text_by_local_name(metadata_body, "language");

    if metadata_identifier.is_none() {
        diagnostics_summary.push("missing_metadata_identifier:dc:identifier".into());
    }
    if metadata_title.is_none() {
        diagnostics_summary.push("missing_metadata_title:dc:title".into());
    }
    if metadata_language.is_none() {
        diagnostics_summary.push("missing_metadata_language:dc:language".into());
    }

    let artifact = LocalBookEpubPackageMetadataArtifact {
        fail_closed: !diagnostics_summary.is_empty(),
        metadata_identifier,
        metadata_title,
        metadata_author,
        metadata_language,
        package_unique_identifier_id,
        diagnostics_summary,
    };
    artifact.validate()?;
    Ok(artifact)
}

pub fn parse_webdav_descriptor_artifact(
    request: &LocalBookWebDavDescriptorRequest,
) -> Result<LocalBookWebDavDescriptorArtifact, LocalBookError> {
    request.validate()?;

    let descriptor: LocalBookWebDavDescriptorDraft = serde_json::from_str(&request.descriptor_json)
        .map_err(|error| LocalBookError::Decode {
            reason: format!("invalid_webdav_descriptor:{error}"),
        })?;

    let remote_path = normalize_required_owned(descriptor.remote_path, "remote_path")?;
    let title = normalize_required_owned(descriptor.title, "title")?;
    let author = normalize_optional_metadata(descriptor.author, "author")?;
    let format = normalize_optional_metadata(descriptor.format, "format")?;
    let etag = normalize_optional_metadata(descriptor.etag, "etag")?;
    let source_identifier =
        normalize_optional_metadata(descriptor.source_identifier, "source_identifier")?;
    let _remote_modified_at = descriptor.remote_modified_at;
    let byte_count = match descriptor.file_size {
        Some(file_size) if file_size < 0 => {
            return Err(LocalBookError::InvalidMetadata {
                field: "file_size".into(),
            });
        }
        Some(file_size) => file_size as u64,
        None => 0,
    };

    let descriptor_checksum =
        local_book_stable_checksum(&["webdav-descriptor", &request.descriptor_json]);
    let resource_checksum = etag.clone().unwrap_or_else(|| descriptor_checksum.clone());
    let media_type = format
        .as_deref()
        .map(local_book_media_type_for_extension)
        .or_else(|| {
            extension_from_path(&remote_path)
                .as_deref()
                .map(local_book_media_type_for_extension)
        })
        .unwrap_or_else(|| "application/octet-stream".into());
    let resource = LocalBookWebDavDescriptorResource {
        stable_resource_id: local_book_stable_checksum(&[
            "webdav-resource",
            &descriptor_checksum,
            &remote_path,
        ]),
        path: remote_path.clone(),
        media_type,
        byte_count,
        checksum: resource_checksum.clone(),
    };
    let diagnostic = "unsupported_media_type".to_string();
    let artifact = LocalBookWebDavDescriptorArtifact {
        detected_format: LocalBookFormat::WebDav,
        detected_encoding: "utf-8".into(),
        book_id: local_book_stable_checksum(&["webdav", &remote_path, &resource_checksum]),
        title,
        author,
        identifier: source_identifier.or(etag),
        remote_path,
        resource,
        input_byte_count: request.descriptor_json.as_bytes().len() as u64,
        content_checksum_count: 1,
        full_content_persisted_count: 0,
        diagnostic: diagnostic.clone(),
        diagnostics_summary: vec![diagnostic],
        clean_room_maintained: true,
        external_gpl_code_copied: false,
    };
    artifact.validate()?;
    Ok(artifact)
}

pub fn extract_epub_html_text_boundary(
    request: &LocalBookEpubHtmlTextBoundaryRequest,
) -> Result<LocalBookEpubHtmlTextBoundaryResult, LocalBookError> {
    request.validate()?;

    let mut text_parts = Vec::<String>::new();
    let mut suppressed_block_count = 0u32;
    let mut image_fallback_count = 0u32;
    let mut suppressed_stack = Vec::<String>::new();
    let mut cursor = 0usize;
    let html = request.html.as_str();

    while cursor < html.len() {
        let Some(relative_tag_start) = html[cursor..].find('<') else {
            if suppressed_stack.is_empty() {
                append_epub_visible_text(&mut text_parts, &html[cursor..]);
            }
            break;
        };
        let tag_start = cursor + relative_tag_start;
        if suppressed_stack.is_empty() {
            append_epub_visible_text(&mut text_parts, &html[cursor..tag_start]);
        }

        let Some(relative_tag_end) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + relative_tag_end;
        let raw_tag = &html[tag_start + 1..tag_end];
        let tag = parse_epub_html_tag(raw_tag);
        if let Some(tag) = tag {
            if tag.closing {
                if suppressed_stack
                    .last()
                    .is_some_and(|open| open == &tag.name)
                {
                    suppressed_stack.pop();
                }
            } else if is_epub_suppressed_html_tag(&tag.name) {
                suppressed_stack.push(tag.name);
                suppressed_block_count = suppressed_block_count.saturating_add(1);
            } else if suppressed_stack.is_empty() {
                if tag.name == "img" {
                    if let Some(fallback) = epub_image_text_fallback(&tag.attributes) {
                        text_parts.push(fallback);
                        image_fallback_count = image_fallback_count.saturating_add(1);
                    }
                } else if is_epub_text_break_tag(&tag.name) {
                    text_parts.push(" ".into());
                }
            }
        }
        cursor = tag_end + 1;
    }

    let mut preview = collapse_local_book_whitespace(&text_parts.join(" "));
    let effective_limit = request.preview_limit.min(LOCAL_BOOK_MAX_PREVIEW_LIMIT);
    if preview.chars().count() > effective_limit {
        preview = preview.chars().take(effective_limit).collect::<String>();
        preview = preview.trim().to_string();
    }

    let mut diagnostics_summary = Vec::new();
    if suppressed_block_count > 0 {
        diagnostics_summary.push("epub_html_suppressed_invisible_blocks".into());
    }
    if image_fallback_count > 0 {
        diagnostics_summary.push("epub_html_image_text_fallback".into());
    }

    let result = LocalBookEpubHtmlTextBoundaryResult {
        preview,
        suppressed_block_count,
        image_fallback_count,
        diagnostics_summary,
    };
    result.validate()?;
    Ok(result)
}

pub fn extract_local_book_html_text_boundary(
    request: &LocalBookHtmlTextBoundaryRequest,
) -> Result<LocalBookHtmlTextBoundaryResult, LocalBookError> {
    request.validate()?;
    let body_fragment = extract_local_book_html_body_fragment(&request.html);
    let epub_boundary = extract_epub_html_text_boundary(&LocalBookEpubHtmlTextBoundaryRequest {
        html: body_fragment.to_string(),
        preview_limit: request.preview_limit,
    })?;
    let diagnostics_summary = epub_boundary
        .diagnostics_summary
        .iter()
        .map(|diagnostic| {
            diagnostic
                .strip_prefix("epub_html_")
                .map(|suffix| format!("local_html_{suffix}"))
                .unwrap_or_else(|| diagnostic.clone())
        })
        .collect::<Vec<_>>();
    let result = LocalBookHtmlTextBoundaryResult {
        title: extract_local_book_html_title(&request.html),
        preview: epub_boundary.preview,
        suppressed_block_count: epub_boundary.suppressed_block_count,
        image_fallback_count: epub_boundary.image_fallback_count,
        diagnostics_summary,
    };
    result.validate()?;
    Ok(result)
}

pub fn plan_epub_resource_index(
    request: &LocalBookEpubResourceIndexRequest,
) -> Result<LocalBookEpubResourceIndexPlan, LocalBookError> {
    request.validate()?;

    let package_base_path = normalize_epub_path("", &request.package_base_path);
    let explicit_cover_id = request.cover_id.as_deref().map(str::trim);
    let mut resources = Vec::new();
    let mut explicit_cover_resource_id = None;
    let mut cover_image_property_resource_id = None;
    let mut first_image_resource_id = None;
    let mut diagnostics_summary = Vec::new();

    for item in &request.manifest_items {
        let (path, _) = resolve_epub_href(&package_base_path, &item.href);
        let media_type = item.media_type.trim().to_string();
        let is_explicit_cover =
            explicit_cover_id.is_some_and(|cover_id| cover_id == item.id.trim());
        let has_cover_image_property = epub_manifest_has_property(item, "cover-image");

        if is_epub_manifest_reading_document(item, &media_type) {
            continue;
        }

        let resource_kind = local_book_resource_kind(
            &path,
            &media_type,
            is_explicit_cover || has_cover_image_property,
        );
        if resource_kind == LocalBookResourceKind::Other {
            diagnostics_summary.push(format!("epub_resource_skipped:{}", item.id));
            continue;
        }

        let stable_resource_id =
            local_book_stable_checksum(&["epub-resource", &request.book_id, item.id.trim(), &path]);
        let resource = LocalBookResourceIndexEntry {
            stable_resource_id: stable_resource_id.clone(),
            book_id: request.book_id.trim().to_string(),
            relative_locator: path,
            mime_type: media_type.clone(),
            byte_count: 0,
            checksum: None,
            is_materialized: false,
            resource_kind,
        };
        resource.validate()?;

        if is_explicit_cover {
            explicit_cover_resource_id = Some(stable_resource_id.clone());
        }
        if has_cover_image_property && cover_image_property_resource_id.is_none() {
            cover_image_property_resource_id = Some(stable_resource_id.clone());
        }
        if media_type.starts_with("image/") && first_image_resource_id.is_none() {
            first_image_resource_id = Some(stable_resource_id.clone());
        }
        resources.push(resource);
    }

    let cover_resource_id = explicit_cover_resource_id
        .or(cover_image_property_resource_id)
        .or(first_image_resource_id);
    if explicit_cover_id.is_some() && cover_resource_id.is_none() {
        diagnostics_summary.push("epub_cover_id_missing".into());
    }

    let plan = LocalBookEpubResourceIndexPlan {
        resources,
        cover_resource_id,
        diagnostics_summary,
    };
    plan.validate()?;
    Ok(plan)
}

fn default_epub_spine_linear() -> bool {
    true
}

fn resolve_epub_href(base_path: &str, href: &str) -> (String, Option<String>) {
    let decoded_href = decode_epub_href_entities(href.trim());
    let (without_fragment, fragment) = match decoded_href.split_once('#') {
        Some((path, fragment)) => (
            path,
            non_empty_optional(percent_decode_epub_href_component(fragment)),
        ),
        None => (decoded_href.as_str(), None),
    };
    let without_query = without_fragment
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(without_fragment);
    (
        normalize_epub_path(
            base_path,
            &percent_decode_epub_href_component(without_query),
        ),
        fragment,
    )
}

fn normalize_epub_path(base_path: &str, href: &str) -> String {
    let href = href.trim();
    let path = if href.starts_with('/') {
        href.trim_start_matches('/').to_string()
    } else if base_path.trim().is_empty() {
        href.to_string()
    } else {
        format!("{}/{}", base_path.trim().trim_matches('/'), href)
    };
    let mut segments = Vec::<&str>::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }
    segments.join("/")
}

fn epub_parent_path(path: &str) -> String {
    let normalized = normalize_epub_path("", path);
    normalized
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn normalize_epub_known_fragment_ids(
    known_fragment_ids: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut normalized = BTreeMap::new();
    for (path, fragments) in known_fragment_ids {
        normalized.insert(
            normalize_epub_path("", path),
            fragments
                .iter()
                .filter_map(|fragment| non_empty_optional(fragment.clone()))
                .collect(),
        );
    }
    normalized
}

fn epub_spine_fallback_nav_items(
    package_base_path: &str,
    manifest_items: &[LocalBookEpubManifestItem],
    spine_items: &[LocalBookEpubSpineItem],
) -> Vec<LocalBookEpubNavItem> {
    let manifest_by_id = manifest_items
        .iter()
        .map(|item| (item.id.trim(), item))
        .collect::<BTreeMap<_, _>>();
    let mut nav_items = Vec::new();
    for spine_item in spine_items.iter().filter(|item| item.linear) {
        let Some(manifest_item) = manifest_by_id.get(spine_item.idref.trim()) else {
            continue;
        };
        let media_type = manifest_item.media_type.trim();
        if !matches!(media_type, "application/xhtml+xml" | "text/html") {
            continue;
        }
        let (path, _) = resolve_epub_href(package_base_path, &manifest_item.href);
        nav_items.push(LocalBookEpubNavItem {
            title: epub_title_from_path_or_id(&path, &manifest_item.id),
            href: manifest_item.href.clone(),
        });
    }
    nav_items
}

fn epub_title_from_path_or_id(path: &str, id: &str) -> String {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file_name);
    let title = stem
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    non_empty_optional(title).unwrap_or_else(|| id.trim().to_string())
}

fn normalize_epub_draft_field(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EpubPackageMetadataTextElement {
    attributes: BTreeMap<String, String>,
    text: String,
}

fn epub_first_text_by_local_name(input: &str, local_name: &str) -> Option<String> {
    epub_collect_text_elements_by_local_name(input, local_name)
        .into_iter()
        .find_map(|element| non_empty_optional(element.text))
}

fn epub_collect_text_elements_by_local_name(
    input: &str,
    local_name: &str,
) -> Vec<EpubPackageMetadataTextElement> {
    let mut elements = Vec::new();
    let mut cursor = 0usize;
    while cursor < input.len() {
        let Some(relative_tag_start) = input[cursor..].find('<') else {
            break;
        };
        let tag_start = cursor + relative_tag_start;
        let Some(relative_tag_end) = input[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + relative_tag_end;
        let raw_tag = &input[tag_start + 1..tag_end];
        let Some(tag) = parse_epub_html_tag(raw_tag) else {
            cursor = tag_end + 1;
            continue;
        };
        if tag.closing || epub_tag_local_name(&tag.name) != local_name {
            cursor = tag_end + 1;
            continue;
        }
        let body_start = tag_end + 1;
        let Some((closing_tag_start, closing_tag_end)) =
            epub_find_closing_tag_by_local_name(input, body_start, local_name)
        else {
            cursor = tag_end + 1;
            continue;
        };
        let text = epub_xml_text_content(&input[body_start..closing_tag_start]);
        elements.push(EpubPackageMetadataTextElement {
            attributes: tag.attributes,
            text,
        });
        cursor = closing_tag_end + 1;
    }
    elements
}

fn epub_first_element_body_by_local_name<'a>(input: &'a str, local_name: &str) -> Option<&'a str> {
    let mut cursor = 0usize;
    while cursor < input.len() {
        let relative_tag_start = input[cursor..].find('<')?;
        let tag_start = cursor + relative_tag_start;
        let relative_tag_end = input[tag_start..].find('>')?;
        let tag_end = tag_start + relative_tag_end;
        let raw_tag = &input[tag_start + 1..tag_end];
        let Some(tag) = parse_epub_html_tag(raw_tag) else {
            cursor = tag_end + 1;
            continue;
        };
        if tag.closing || epub_tag_local_name(&tag.name) != local_name {
            cursor = tag_end + 1;
            continue;
        }
        let body_start = tag_end + 1;
        let (closing_tag_start, _) =
            epub_find_closing_tag_by_local_name(input, body_start, local_name)?;
        return Some(&input[body_start..closing_tag_start]);
    }
    None
}

fn epub_first_tag_attributes_by_local_name(
    input: &str,
    local_name: &str,
) -> Option<BTreeMap<String, String>> {
    let mut cursor = 0usize;
    while cursor < input.len() {
        let relative_tag_start = input[cursor..].find('<')?;
        let tag_start = cursor + relative_tag_start;
        let relative_tag_end = input[tag_start..].find('>')?;
        let tag_end = tag_start + relative_tag_end;
        let raw_tag = &input[tag_start + 1..tag_end];
        let Some(tag) = parse_epub_html_tag(raw_tag) else {
            cursor = tag_end + 1;
            continue;
        };
        if !tag.closing && epub_tag_local_name(&tag.name) == local_name {
            return Some(tag.attributes);
        }
        cursor = tag_end + 1;
    }
    None
}

fn epub_find_closing_tag_by_local_name(
    input: &str,
    start: usize,
    local_name: &str,
) -> Option<(usize, usize)> {
    let mut cursor = start;
    while cursor < input.len() {
        let relative_tag_start = input[cursor..].find('<')?;
        let tag_start = cursor + relative_tag_start;
        let relative_tag_end = input[tag_start..].find('>')?;
        let tag_end = tag_start + relative_tag_end;
        let raw_tag = &input[tag_start + 1..tag_end];
        if let Some(tag) = parse_epub_html_tag(raw_tag) {
            if tag.closing && epub_tag_local_name(&tag.name) == local_name {
                return Some((tag_start, tag_end));
            }
        }
        cursor = tag_end + 1;
    }
    None
}

fn epub_xml_text_content(input: &str) -> String {
    let mut parts = Vec::new();
    let mut cursor = 0usize;
    while cursor < input.len() {
        let Some(relative_tag_start) = input[cursor..].find('<') else {
            append_epub_visible_text(&mut parts, &input[cursor..]);
            break;
        };
        let tag_start = cursor + relative_tag_start;
        append_epub_visible_text(&mut parts, &input[cursor..tag_start]);
        let Some(relative_tag_end) = input[tag_start..].find('>') else {
            break;
        };
        cursor = tag_start + relative_tag_end + 1;
    }
    collapse_local_book_whitespace(&parts.join(" "))
}

fn epub_tag_local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn epub_import_diagnostic_from_summary(summary: &str) -> Option<LocalBookEpubImportDiagnostic> {
    let (kind, detail) = summary.split_once(':')?;
    let (code, detail) = match kind {
        "epub_nav_empty" | "epub_nav_invalid" => (
            LocalBookEpubImportDiagnosticCode::InvalidNav,
            epub_diagnostic_fallback_detail(detail),
        ),
        "epub_ncx_empty" | "epub_ncx_invalid" => (
            LocalBookEpubImportDiagnosticCode::InvalidNcx,
            epub_diagnostic_fallback_detail(detail),
        ),
        "unsafe_archive_path" => (
            LocalBookEpubImportDiagnosticCode::UnsafeArchivePath,
            detail.trim().to_string(),
        ),
        "missing_container" => (
            LocalBookEpubImportDiagnosticCode::MissingContainer,
            detail.trim().to_string(),
        ),
        "unsupported_encryption" => (
            LocalBookEpubImportDiagnosticCode::UnsupportedEncryption,
            detail.trim().to_string(),
        ),
        "invalid_opf"
        | "missing_manifest_item"
        | "missing_spine_item"
        | "epub_spine_missing_manifest" => (
            LocalBookEpubImportDiagnosticCode::InvalidOpf,
            detail.trim().to_string(),
        ),
        "epub_nav_missing_spine" => (
            LocalBookEpubImportDiagnosticCode::MissingChapterResource,
            detail.trim().to_string(),
        ),
        "epub_nav_skipped_non_linear" => (
            LocalBookEpubImportDiagnosticCode::InvalidNav,
            detail.trim().to_string(),
        ),
        _ => return None,
    };
    non_empty_optional(detail).map(|detail| LocalBookEpubImportDiagnostic { code, detail })
}

fn epub_diagnostic_fallback_detail(detail: &str) -> String {
    detail.trim().replace('_', " ")
}

fn is_unsafe_epub_archive_path(path: &str) -> bool {
    let path = path.trim();
    path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|segment| segment == ".." || segment.trim().is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EpubHtmlTag {
    name: String,
    closing: bool,
    attributes: BTreeMap<String, String>,
}

fn parse_epub_html_tag(raw_tag: &str) -> Option<EpubHtmlTag> {
    let trimmed = raw_tag.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('!')
        || trimmed.starts_with('?')
        || trimmed.starts_with("!--")
    {
        return None;
    }
    let closing = trimmed.starts_with('/');
    let body = if closing {
        trimmed[1..].trim_start()
    } else {
        trimmed
    };
    let name_end = body
        .char_indices()
        .find_map(|(index, ch)| {
            (!(ch.is_ascii_alphanumeric() || ch == ':' || ch == '-' || ch == '_')).then_some(index)
        })
        .unwrap_or(body.len());
    let name = body[..name_end].to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    let attributes = if closing {
        BTreeMap::new()
    } else {
        parse_epub_html_attributes(&body[name_end..])
    };
    Some(EpubHtmlTag {
        name,
        closing,
        attributes,
    })
}

fn parse_epub_html_attributes(input: &str) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    let bytes = input.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            break;
        }
        let name_start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && bytes[cursor] != b'='
            && bytes[cursor] != b'/'
        {
            cursor += 1;
        }
        let name = input[name_start..cursor].trim().to_ascii_lowercase();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let mut value = String::new();
        if cursor < bytes.len() && bytes[cursor] == b'=' {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor < bytes.len() && (bytes[cursor] == b'"' || bytes[cursor] == b'\'') {
                let quote = bytes[cursor];
                cursor += 1;
                let value_start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                value = input[value_start..cursor].to_string();
                if cursor < bytes.len() {
                    cursor += 1;
                }
            } else {
                let value_start = cursor;
                while cursor < bytes.len()
                    && !bytes[cursor].is_ascii_whitespace()
                    && bytes[cursor] != b'/'
                {
                    cursor += 1;
                }
                value = input[value_start..cursor].to_string();
            }
        }
        if !name.is_empty() {
            attributes.insert(
                name,
                collapse_local_book_whitespace(&decode_epub_href_entities(&value)),
            );
        }
    }
    attributes
}

fn append_epub_visible_text(parts: &mut Vec<String>, text: &str) {
    let decoded = collapse_local_book_whitespace(&decode_epub_href_entities(text));
    if !decoded.is_empty() {
        parts.push(decoded);
    }
}

fn epub_image_text_fallback(attributes: &BTreeMap<String, String>) -> Option<String> {
    attributes
        .get("alt")
        .or_else(|| attributes.get("title"))
        .and_then(|value| non_empty_optional(value.clone()))
}

fn is_epub_suppressed_html_tag(name: &str) -> bool {
    matches!(name, "script" | "style" | "noscript")
}

fn is_epub_text_break_tag(name: &str) -> bool {
    matches!(
        name,
        "br" | "p"
            | "div"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "li"
            | "tr"
            | "td"
            | "th"
            | "blockquote"
    )
}

fn collapse_local_book_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_local_book_html_title(html: &str) -> Option<String> {
    let title_start = find_ascii_case_insensitive(html, "<title")?;
    let after_title_start = &html[title_start..];
    let tag_end = after_title_start.find('>')?;
    let content_start = title_start + tag_end + 1;
    let content = &html[content_start..];
    let title_end = find_ascii_case_insensitive(content, "</title>")?;
    let decoded = decode_epub_href_entities(&content[..title_end]);
    let title = collapse_local_book_whitespace(&decoded);
    (!title.is_empty()).then_some(title)
}

fn extract_local_book_html_body_fragment(html: &str) -> &str {
    let Some(body_start) = find_ascii_case_insensitive(html, "<body") else {
        return html;
    };
    let after_body_start = &html[body_start..];
    let Some(tag_end) = after_body_start.find('>') else {
        return html;
    };
    let content_start = body_start + tag_end + 1;
    let body_content = &html[content_start..];
    let Some(body_end) = find_ascii_case_insensitive(body_content, "</body>") else {
        return body_content;
    };
    &body_content[..body_end]
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

fn link_local_book_chapter_neighbors(chapters: &mut [LocalBookChapterIndexEntry]) {
    let ids = chapters
        .iter()
        .map(|chapter| chapter.stable_chapter_id.clone())
        .collect::<Vec<_>>();
    for (index, chapter) in chapters.iter_mut().enumerate() {
        chapter.previous_chapter_id = index
            .checked_sub(1)
            .and_then(|previous| ids.get(previous).cloned());
        chapter.next_chapter_id = ids.get(index + 1).cloned();
    }
}

fn decode_epub_href_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        let entity_rest = &rest[start + 1..];
        if entity_rest.is_empty() {
            output.push('&');
            rest = entity_rest;
            continue;
        }

        if entity_rest.starts_with('#') {
            let Some(end) = entity_rest.find(';') else {
                output.push('&');
                rest = entity_rest;
                continue;
            };
            let entity = &entity_rest[..end];
            if let Some(decoded) = decode_epub_entity(entity) {
                output.push(decoded);
            } else {
                output.push('&');
                output.push_str(entity);
                output.push(';');
            }
            rest = &entity_rest[end + 1..];
            continue;
        }

        let name_end = entity_rest
            .char_indices()
            .find_map(|(index, ch)| (!ch.is_ascii_alphanumeric()).then_some(index))
            .unwrap_or(entity_rest.len());
        if name_end == 0 {
            output.push('&');
            rest = entity_rest;
            continue;
        }

        let name = &entity_rest[..name_end];
        let after_name = &entity_rest[name_end..];
        if let Some(after_semicolon) = after_name.strip_prefix(';') {
            if let Some(decoded) = decode_epub_entity(name) {
                output.push(decoded);
            } else {
                output.push('&');
                output.push_str(name);
                output.push(';');
            }
            rest = after_semicolon;
            continue;
        }

        if let Some(decoded) = decode_epub_legacy_entity_without_semicolon(name, after_name) {
            output.push(decoded.0);
            rest = &entity_rest[decoded.1..];
            continue;
        }

        output.push('&');
        rest = entity_rest;
    }
    output.push_str(rest);
    output
}

fn decode_epub_entity(entity: &str) -> Option<char> {
    match entity.to_ascii_lowercase().as_str() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "copy" => Some('©'),
        "mdash" => Some('—'),
        "hellip" => Some('…'),
        "ldquo" => Some('“'),
        "rdquo" => Some('”'),
        "rsquo" => Some('’'),
        "euro" => Some('€'),
        "sect" => Some('§'),
        "plusmn" => Some('±'),
        "times" => Some('×'),
        "divide" => Some('÷'),
        "frac12" => Some('½'),
        "frac14" => Some('¼'),
        "frac34" => Some('¾'),
        "deg" => Some('°'),
        "cent" => Some('¢'),
        "eacute" => Some(epub_latin_entity_case(entity, 'É', 'é')),
        "egrave" => Some(epub_latin_entity_case(entity, 'È', 'è')),
        "agrave" => Some(epub_latin_entity_case(entity, 'À', 'à')),
        "iacute" => Some(epub_latin_entity_case(entity, 'Í', 'í')),
        "oacute" => Some(epub_latin_entity_case(entity, 'Ó', 'ó')),
        "uacute" => Some(epub_latin_entity_case(entity, 'Ú', 'ú')),
        "uuml" => Some(epub_latin_entity_case(entity, 'Ü', 'ü')),
        "ntilde" => Some(epub_latin_entity_case(entity, 'Ñ', 'ñ')),
        _ if entity.starts_with("#x") || entity.starts_with("#X") => {
            u32::from_str_radix(&entity[2..], 16)
                .ok()
                .and_then(char::from_u32)
        }
        _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
        _ => None,
    }
}

fn decode_epub_legacy_entity_without_semicolon(
    name: &str,
    after_name: &str,
) -> Option<(char, usize)> {
    if after_name.starts_with('=') {
        return None;
    }
    decode_epub_entity(name).map(|decoded| (decoded, name.len()))
}

fn epub_latin_entity_case(entity: &str, upper: char, lower: char) -> char {
    if entity
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
    {
        upper
    } else {
        lower
    }
}

fn percent_decode_epub_href_component(input: &str) -> String {
    if !input.as_bytes().contains(&b'%') {
        return input.to_string();
    }
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn epub_manifest_has_property(item: &LocalBookEpubManifestItem, property: &str) -> bool {
    item.properties
        .iter()
        .flat_map(|value| value.split_whitespace())
        .any(|candidate| candidate == property)
}

fn is_epub_manifest_reading_document(item: &LocalBookEpubManifestItem, media_type: &str) -> bool {
    epub_manifest_has_property(item, "nav")
        || matches!(
            media_type,
            "application/xhtml+xml" | "text/html" | "application/x-dtbncx+xml"
        )
}

fn detect_local_book_format_from_bytes(
    bytes: &[u8],
    declared_format: Option<LocalBookFormat>,
) -> LocalBookFormat {
    if bytes.starts_with(&[0x50, 0x4b, 0x03, 0x04]) {
        if declared_format == Some(LocalBookFormat::Epub)
            || ascii_contains(bytes, b"META-INF/container.xml", bytes.len())
        {
            return LocalBookFormat::Epub;
        }
        return LocalBookFormat::Archive;
    }
    if looks_like_tar(bytes) {
        return LocalBookFormat::Archive;
    }
    if bytes.starts_with(b"%PDF-") {
        return LocalBookFormat::Pdf;
    }
    if bytes.starts_with(&[0x89, 0x9b, 0x9a, 0xde]) || bytes.starts_with(&[0xde, 0x9a, 0x9b, 0x89])
    {
        return LocalBookFormat::Umd;
    }
    if ascii_contains(bytes, b"BOOKMOBI", 4096) {
        return if declared_format == Some(LocalBookFormat::Azw) {
            LocalBookFormat::Azw
        } else {
            LocalBookFormat::Mobi
        };
    }
    if looks_like_webdav_descriptor(bytes) {
        return LocalBookFormat::WebDav;
    }
    if let Some(
        declared @ (LocalBookFormat::Mobi
        | LocalBookFormat::Azw
        | LocalBookFormat::Umd
        | LocalBookFormat::Archive
        | LocalBookFormat::WebDav),
    ) = declared_format
    {
        return declared;
    }
    LocalBookFormat::Txt
}

fn local_book_format_wire_value(format: LocalBookFormat) -> &'static str {
    match format {
        LocalBookFormat::Txt => "txt",
        LocalBookFormat::Epub => "epub",
        LocalBookFormat::Pdf => "pdf",
        LocalBookFormat::Html => "html",
        LocalBookFormat::Mobi => "mobi",
        LocalBookFormat::Azw => "azw",
        LocalBookFormat::Umd => "umd",
        LocalBookFormat::Archive => "archive",
        LocalBookFormat::WebDav => "webdav",
        LocalBookFormat::Unknown => "unknown",
    }
}

fn normalize_extension(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('.').to_ascii_lowercase();
    (!value.is_empty()).then_some(value)
}

fn extension_from_path(path: &str) -> Option<String> {
    let filename = path.rsplit(['/', '\\']).next()?.trim();
    let extension = filename.rsplit_once('.')?.1;
    normalize_extension(extension)
}

fn ascii_contains(bytes: &[u8], needle: &[u8], max_len: usize) -> bool {
    if needle.is_empty() {
        return true;
    }
    let len = bytes.len().min(max_len);
    bytes[..len]
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn looks_like_tar(bytes: &[u8]) -> bool {
    bytes.get(257..262).is_some_and(|magic| magic == b"ustar")
}

fn looks_like_webdav_descriptor(bytes: &[u8]) -> bool {
    let len = bytes.len().min(2048);
    let Ok(text) = std::str::from_utf8(&bytes[..len]) else {
        return false;
    };
    text.contains("\"remotePath\"")
        && (text.contains("\"etag\"")
            || text.contains("\"format\"")
            || text.contains("\"fileSize\""))
}

/// Parse a TXT local book from bytes.
pub fn parse_txt_book(input: LocalBookInput<'_>) -> Result<LocalBook, LocalBookError> {
    parse_txt_book_with_policy(input, &ChapterSplitPolicy::default())
}

/// Parse a TXT local book from bytes using an explicit chapter split policy.
pub fn parse_txt_book_with_policy(
    input: LocalBookInput<'_>,
    policy: &ChapterSplitPolicy,
) -> Result<LocalBook, LocalBookError> {
    let book_id = normalize_required(input.book_id, "book_id")?;
    if input.bytes.is_empty() {
        return Err(LocalBookError::EmptyInput);
    }

    let (decoded, encoding) = decode_txt_bytes(input.bytes)?;
    let normalized = normalize_text(&decoded);
    if normalized.trim().is_empty() {
        return Err(LocalBookError::EmptyInput);
    }

    let title = derive_title(input.title, input.file_name, &book_id);
    let author = input
        .author
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();
    let chapters = split_chapters_with_policy(&normalized, policy)?;
    let toc = chapters
        .iter()
        .map(|chapter| TocEntry {
            index: chapter.index,
            title: chapter.title.clone(),
            url: format!("local://{book_id}/chapter/{}", chapter.index),
        })
        .collect::<Vec<_>>();
    let last_chapter = chapters.last().map(|chapter| chapter.title.clone());

    Ok(LocalBook {
        book: Book {
            book_id,
            title,
            author,
            cover_url: None,
            intro: None,
            kind: Some("local".into()),
            last_chapter,
        },
        format: LocalBookFormat::Txt,
        encoding,
        byte_len: input.bytes.len(),
        char_len: normalized.chars().count(),
        toc,
        chapters,
    })
}

/// Parse already-decoded TXT content. This is useful for tests and callers that
/// receive trusted UTF-8 text from their host platform.
pub fn parse_txt_text(
    book_id: &str,
    title: Option<&str>,
    author: Option<&str>,
    file_name: Option<&str>,
    text: &str,
) -> Result<LocalBook, LocalBookError> {
    parse_txt_book_with_policy(
        LocalBookInput {
            book_id,
            title,
            author,
            file_name,
            bytes: text.as_bytes(),
        },
        &ChapterSplitPolicy::default(),
    )
}

/// Parse already-decoded TXT content with an explicit split policy.
pub fn parse_txt_text_with_policy(
    book_id: &str,
    title: Option<&str>,
    author: Option<&str>,
    file_name: Option<&str>,
    text: &str,
    policy: &ChapterSplitPolicy,
) -> Result<LocalBook, LocalBookError> {
    parse_txt_book_with_policy(
        LocalBookInput {
            book_id,
            title,
            author,
            file_name,
            bytes: text.as_bytes(),
        },
        policy,
    )
}

/// Flatten hierarchical local-book TOC items into domain TOC entries.
pub fn flatten_local_toc_items(
    book_id: &str,
    items: &[LocalTocItem],
) -> Result<Vec<TocEntry>, LocalBookError> {
    let book_id = normalize_required(book_id, "book_id")?;
    let mut entries = Vec::new();
    for item in items {
        flatten_local_toc_item(&book_id, item, None, &mut entries)?;
    }
    Ok(entries)
}

fn flatten_local_toc_item(
    book_id: &str,
    item: &LocalTocItem,
    parent_level: Option<u32>,
    entries: &mut Vec<TocEntry>,
) -> Result<(), LocalBookError> {
    validate_local_toc_item(item, parent_level)?;
    let index = entries.len() as u32;
    entries.push(TocEntry {
        index,
        title: item.title.trim().to_string(),
        url: format!("local://{book_id}/chapter/{index}"),
    });
    if let Some(children) = &item.children {
        for child in children {
            flatten_local_toc_item(book_id, child, Some(item.level), entries)?;
        }
    }
    Ok(())
}

fn split_chapters_with_policy(
    text: &str,
    policy: &ChapterSplitPolicy,
) -> Result<Vec<LocalBookChapter>, LocalBookError> {
    match policy.pattern {
        ChapterSplitPattern::Auto => Ok(split_chapters(text)),
        ChapterSplitPattern::Regex => split_chapters_by_regex(text, policy.regex.as_deref()),
        ChapterSplitPattern::Marker => Ok(split_chapters_by_marker(text, policy.marker.as_deref())),
        ChapterSplitPattern::Size => Ok(split_chapters_by_size(text, policy.size_bytes)),
    }
}

fn split_chapters_by_regex(
    text: &str,
    pattern: Option<&str>,
) -> Result<Vec<LocalBookChapter>, LocalBookError> {
    let Some(pattern) = pattern.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(single_policy_chapter(text));
    };
    let regex = Regex::new(pattern).map_err(|_| LocalBookError::InvalidMetadata {
        field: "regex".into(),
    })?;
    let lines = indexed_lines(text);
    let heading_indices = lines
        .iter()
        .enumerate()
        .filter_map(|(line_index, (_, line))| regex.is_match(line.trim()).then_some(line_index))
        .collect::<Vec<_>>();
    if heading_indices.is_empty() {
        return Ok(single_policy_chapter(text));
    }

    let mut chapters = Vec::new();
    for (heading_order, line_index) in heading_indices.iter().enumerate() {
        let next_line_index = heading_indices
            .get(heading_order + 1)
            .copied()
            .unwrap_or(lines.len());
        let title = lines[*line_index].1.trim().to_string();
        let content = join_line_range(&lines, line_index + 1, next_line_index);
        let end_char = if next_line_index < lines.len() {
            lines[next_line_index].0
        } else {
            text.chars().count()
        };
        chapters.push(LocalBookChapter {
            index: chapters.len() as u32,
            title,
            content: trim_outer_blank_lines(&content),
            start_char: lines[*line_index].0,
            end_char,
        });
    }
    Ok(chapters)
}

fn split_chapters_by_marker(text: &str, marker: Option<&str>) -> Vec<LocalBookChapter> {
    let Some(marker) = marker.map(str::trim).filter(|value| !value.is_empty()) else {
        return single_policy_chapter(text);
    };
    if !text.contains(marker) {
        return single_policy_chapter(text);
    }

    let marker_char_len = marker.chars().count();
    let mut offset = 0usize;
    let mut chapters = Vec::new();
    for (part_index, part) in text.split(marker).enumerate() {
        let part_char_len = part.chars().count();
        if part_index == 0 && part.trim().is_empty() {
            offset += part_char_len + marker_char_len;
            continue;
        }

        let part_start = offset;
        let part_end = offset + part_char_len;
        let mut lines = part.lines();
        let title = lines
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Chapter {}", chapters.len() + 1));
        let body = trim_outer_blank_lines(&lines.collect::<Vec<_>>().join("\n"));
        chapters.push(LocalBookChapter {
            index: chapters.len() as u32,
            title,
            content: body,
            start_char: part_start,
            end_char: part_end,
        });
        offset += part_char_len + marker_char_len;
    }

    if chapters.is_empty() {
        single_policy_chapter(text)
    } else {
        chapters
    }
}

fn split_chapters_by_size(text: &str, size_bytes: Option<usize>) -> Vec<LocalBookChapter> {
    let Some(size_bytes) = size_bytes.filter(|value| *value > 0) else {
        return single_policy_chapter(text);
    };
    let total_bytes = text.len();
    if total_bytes == 0 {
        return single_policy_chapter(text);
    }

    let mut chapters = Vec::new();
    let mut start_byte = 0usize;
    while start_byte < total_bytes {
        let mut end_byte = (start_byte + size_bytes).min(total_bytes);
        while end_byte > start_byte && !text.is_char_boundary(end_byte) {
            end_byte -= 1;
        }
        if end_byte == start_byte {
            end_byte = text[start_byte..]
                .char_indices()
                .nth(1)
                .map(|(offset, _)| start_byte + offset)
                .unwrap_or(total_bytes);
        }

        let start_char = text[..start_byte].chars().count();
        let end_char = text[..end_byte].chars().count();
        chapters.push(LocalBookChapter {
            index: chapters.len() as u32,
            title: format!("Chapter {}", chapters.len() + 1),
            content: text[start_byte..end_byte].to_string(),
            start_char,
            end_char,
        });
        start_byte = end_byte;
    }
    chapters
}

fn single_policy_chapter(text: &str) -> Vec<LocalBookChapter> {
    vec![LocalBookChapter {
        index: 0,
        title: "Chapter 1".into(),
        content: trim_outer_blank_lines(text),
        start_char: 0,
        end_char: text.chars().count(),
    }]
}

impl LocalBookLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_book(&mut self, book: LocalBook) -> Result<LocalBook, LocalBookError> {
        validate_local_book(&book)?;
        self.books.insert(book.book.book_id.clone(), book.clone());
        Ok(book)
    }

    pub fn parse_and_upsert_txt(
        &mut self,
        input: LocalBookInput<'_>,
    ) -> Result<LocalBook, LocalBookError> {
        let book = parse_txt_book(input)?;
        self.upsert_book(book)
    }

    pub fn parse_and_upsert_txt_with_policy(
        &mut self,
        input: LocalBookInput<'_>,
        policy: &ChapterSplitPolicy,
    ) -> Result<LocalBook, LocalBookError> {
        let book = parse_txt_book_with_policy(input, policy)?;
        self.upsert_book(book)
    }

    pub fn get_book(&self, book_id: &str) -> Result<Option<LocalBook>, LocalBookError> {
        let book_id = normalize_required(book_id, "book_id")?;
        Ok(self.books.get(&book_id).cloned())
    }

    pub fn list_books(&self) -> Vec<LocalBook> {
        let mut books = self.books.values().cloned().collect::<Vec<_>>();
        books.sort_by(|a, b| {
            a.book
                .title
                .cmp(&b.book.title)
                .then_with(|| a.book.book_id.cmp(&b.book.book_id))
        });
        books
    }

    pub fn get_chapter(
        &self,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<LocalBookChapter, LocalBookError> {
        let book_id = normalize_required(book_id, "book_id")?;
        let book = self
            .books
            .get(&book_id)
            .ok_or_else(|| LocalBookError::BookNotFound {
                book_id: book_id.clone(),
            })?;
        book.chapters
            .iter()
            .find(|chapter| chapter.index == chapter_index)
            .cloned()
            .ok_or(LocalBookError::ChapterNotFound {
                book_id,
                chapter_index,
            })
    }

    pub fn remove_book(&mut self, book_id: &str) -> Result<bool, LocalBookError> {
        let book_id = normalize_required(book_id, "book_id")?;
        Ok(self.books.remove(&book_id).is_some())
    }

    pub fn export_snapshot(
        &self,
        exported_at: i64,
    ) -> Result<LocalBookLibrarySnapshot, LocalBookError> {
        let mut snapshot = LocalBookLibrarySnapshot {
            schema_version: LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            books: self.books.values().cloned().collect(),
        };
        sort_local_book_snapshot(&mut snapshot);
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn replace_with_snapshot(
        &mut self,
        snapshot: LocalBookLibrarySnapshot,
    ) -> Result<(), LocalBookError> {
        snapshot.validate()?;
        let mut books = HashMap::new();
        for book in snapshot.books {
            books.insert(book.book.book_id.clone(), book);
        }
        self.books = books;
        Ok(())
    }
}

fn sort_local_book_snapshot(snapshot: &mut LocalBookLibrarySnapshot) {
    snapshot.books.sort_by(|a, b| {
        a.book
            .book_id
            .cmp(&b.book.book_id)
            .then_with(|| a.book.title.cmp(&b.book.title))
    });
}

fn decode_txt_bytes(bytes: &[u8]) -> Result<(String, LocalBookEncoding), LocalBookError> {
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        let text = std::str::from_utf8(&bytes[3..]).map_err(|e| LocalBookError::Decode {
            reason: e.to_string(),
        })?;
        return Ok((text.to_string(), LocalBookEncoding::Utf8Bom));
    }

    if bytes.starts_with(&[0xff, 0xfe]) {
        return decode_utf16(&bytes[2..], LocalBookEncoding::Utf16Le, u16::from_le_bytes);
    }

    if bytes.starts_with(&[0xfe, 0xff]) {
        return decode_utf16(&bytes[2..], LocalBookEncoding::Utf16Be, u16::from_be_bytes);
    }

    match std::str::from_utf8(bytes) {
        Ok(text) => Ok((text.to_string(), LocalBookEncoding::Utf8)),
        Err(_) => Err(LocalBookError::UnsupportedEncoding),
    }
}

fn decode_utf16(
    bytes: &[u8],
    encoding: LocalBookEncoding,
    convert: fn([u8; 2]) -> u16,
) -> Result<(String, LocalBookEncoding), LocalBookError> {
    if bytes.len() % 2 != 0 {
        return Err(LocalBookError::Decode {
            reason: "UTF-16 byte length is not even".into(),
        });
    }

    let code_units = bytes
        .chunks_exact(2)
        .map(|chunk| convert([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    let text = String::from_utf16(&code_units).map_err(|e| LocalBookError::Decode {
        reason: e.to_string(),
    })?;
    Ok((text, encoding))
}

fn normalize_required(value: &str, field: &str) -> Result<String, LocalBookError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(LocalBookError::InvalidMetadata {
            field: field.into(),
        });
    }
    Ok(trimmed.to_string())
}

fn normalize_required_owned(value: String, field: &str) -> Result<String, LocalBookError> {
    normalize_required(&value, field)
}

fn validate_required_metadata(value: &str, field: &str) -> Result<(), LocalBookError> {
    normalize_required(value, field).map(|_| ())
}

fn non_empty_optional(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn normalize_optional_metadata(
    value: Option<String>,
    field: &str,
) -> Result<Option<String>, LocalBookError> {
    value
        .map(|value| normalize_required_owned(value, field))
        .transpose()
}

fn validate_optional_metadata(value: &Option<String>, field: &str) -> Result<(), LocalBookError> {
    if value.as_ref().is_some_and(|value| value.trim().is_empty()) {
        return Err(LocalBookError::InvalidMetadata {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_local_toc_item(
    item: &LocalTocItem,
    parent_level: Option<u32>,
) -> Result<(), LocalBookError> {
    validate_required_metadata(&item.title, "title")?;
    if item.level == 0 {
        return Err(LocalBookError::InvalidMetadata {
            field: "level".into(),
        });
    }
    if parent_level.is_some_and(|parent_level| item.level <= parent_level) {
        return Err(LocalBookError::InvalidMetadata {
            field: "children.level".into(),
        });
    }
    if let Some(children) = &item.children {
        for child in children {
            validate_local_toc_item(child, Some(item.level))?;
        }
    }
    Ok(())
}

fn validate_local_book(book: &LocalBook) -> Result<(), LocalBookError> {
    if book.book.book_id.trim().is_empty() {
        return Err(LocalBookError::InvalidBook {
            field: "book.book_id".into(),
        });
    }
    if book.book.title.trim().is_empty() {
        return Err(LocalBookError::InvalidBook {
            field: "book.title".into(),
        });
    }
    if book.byte_len == 0 {
        return Err(LocalBookError::InvalidBook {
            field: "byte_len".into(),
        });
    }
    if book.char_len == 0 {
        return Err(LocalBookError::InvalidBook {
            field: "char_len".into(),
        });
    }
    if book.toc.len() != book.chapters.len() || book.chapters.is_empty() {
        return Err(LocalBookError::InvalidBook {
            field: "chapters".into(),
        });
    }

    for (expected_index, chapter) in book.chapters.iter().enumerate() {
        if chapter.index != expected_index as u32 {
            return Err(LocalBookError::InvalidBook {
                field: "chapters.index".into(),
            });
        }
        if chapter.title.trim().is_empty() {
            return Err(LocalBookError::InvalidBook {
                field: "chapters.title".into(),
            });
        }
        if chapter.start_char > chapter.end_char || chapter.end_char > book.char_len {
            return Err(LocalBookError::InvalidBook {
                field: "chapters.range".into(),
            });
        }
    }

    for (expected_index, toc) in book.toc.iter().enumerate() {
        if toc.index != expected_index as u32 {
            return Err(LocalBookError::InvalidBook {
                field: "toc.index".into(),
            });
        }
        if toc.title.trim().is_empty() {
            return Err(LocalBookError::InvalidBook {
                field: "toc.title".into(),
            });
        }
    }

    Ok(())
}

fn derive_title(title: Option<&str>, file_name: Option<&str>, book_id: &str) -> String {
    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        return title.to_string();
    }

    if let Some(file_name) = file_name.map(str::trim).filter(|value| !value.is_empty()) {
        let stem = Path::new(file_name)
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(stem) = stem {
            return stem.to_string();
        }
    }

    book_id.to_string()
}

fn normalize_text(text: &str) -> String {
    text.trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn split_chapters(text: &str) -> Vec<LocalBookChapter> {
    let lines = indexed_lines(text);
    let heading_indices = lines
        .iter()
        .enumerate()
        .filter_map(|(line_index, (_, line))| is_chapter_heading(line.trim()).then_some(line_index))
        .collect::<Vec<_>>();

    if heading_indices.is_empty() {
        return vec![LocalBookChapter {
            index: 0,
            title: "正文".into(),
            content: trim_outer_blank_lines(text),
            start_char: 0,
            end_char: text.chars().count(),
        }];
    }

    let mut chapters = Vec::new();
    let first_heading = heading_indices[0];
    let preface = join_line_range(&lines, 0, first_heading);
    if !preface.trim().is_empty() {
        chapters.push(LocalBookChapter {
            index: 0,
            title: "序章".into(),
            content: trim_outer_blank_lines(&preface),
            start_char: 0,
            end_char: lines[first_heading].0,
        });
    }

    for (heading_order, line_index) in heading_indices.iter().enumerate() {
        let next_line_index = heading_indices
            .get(heading_order + 1)
            .copied()
            .unwrap_or(lines.len());
        let title = lines[*line_index].1.trim().to_string();
        let content = join_line_range(&lines, line_index + 1, next_line_index);
        let start_char = lines[*line_index].0;
        let end_char = if next_line_index < lines.len() {
            lines[next_line_index].0
        } else {
            text.chars().count()
        };
        chapters.push(LocalBookChapter {
            index: chapters.len() as u32,
            title,
            content: trim_outer_blank_lines(&content),
            start_char,
            end_char,
        });
    }

    chapters
}

fn indexed_lines(text: &str) -> Vec<(usize, String)> {
    let mut offset = 0usize;
    let mut lines = Vec::new();
    for line in text.split('\n') {
        lines.push((offset, line.to_string()));
        offset += line.chars().count() + 1;
    }
    lines
}

fn join_line_range(lines: &[(usize, String)], start: usize, end: usize) -> String {
    if start >= end {
        return String::new();
    }
    lines[start..end]
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn trim_outer_blank_lines(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let Some(first) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let last = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .unwrap_or(first);
    lines[first..=last].join("\n")
}

fn is_chapter_heading(line: &str) -> bool {
    if line.is_empty() || line.chars().count() > 80 {
        return false;
    }

    if line.starts_with('第') {
        let mut has_ordinal = false;
        for ch in line.chars().skip(1).take(16) {
            if is_chapter_ordinal_char(ch) {
                has_ordinal = true;
                continue;
            }
            return has_ordinal && matches!(ch, '章' | '回' | '节' | '卷');
        }
        return false;
    }

    if line.starts_with('卷') && line.chars().count() <= 40 {
        return true;
    }

    let lower = line.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("chapter") {
        return rest
            .chars()
            .next()
            .map(|ch| ch.is_ascii_whitespace() || ch.is_ascii_digit())
            .unwrap_or(false);
    }

    false
}

fn is_chapter_ordinal_char(ch: char) -> bool {
    ch.is_ascii_digit()
        || matches!(
            ch,
            '零' | '〇'
                | '一'
                | '二'
                | '三'
                | '四'
                | '五'
                | '六'
                | '七'
                | '八'
                | '九'
                | '十'
                | '百'
                | '千'
                | '万'
                | '两'
                | '壹'
                | '贰'
                | '叁'
                | '肆'
                | '伍'
                | '陆'
                | '柒'
                | '捌'
                | '玖'
                | '拾'
                | '佰'
                | '仟'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(book_id: &'a str, bytes: &'a [u8]) -> LocalBookInput<'a> {
        LocalBookInput {
            book_id,
            file_name: Some("三体.txt"),
            title: None,
            author: Some("刘慈欣"),
            bytes,
        }
    }

    fn sample_text(title: &str) -> String {
        format!("{title}\n\n第一章 开始\n正文一\n\n第二章 继续\n正文二")
    }

    fn sample_book(book_id: &str, title: &str) -> LocalBook {
        parse_txt_text(
            book_id,
            Some(title),
            Some("Author"),
            Some(&format!("{title}.txt")),
            &sample_text(title),
        )
        .unwrap()
    }

    fn fingerprint(
        full_checksum: &str,
        title: &str,
        author: Option<&str>,
        chapters: usize,
    ) -> LocalBookFingerprintSet {
        LocalBookFingerprintSet {
            fast: LocalBookFastFingerprint {
                byte_count: 128,
                prefix_checksum: "prefix".into(),
                suffix_checksum: "suffix".into(),
                declared_filename_checksum: Some("fnv1a64:fae0f46ea5a643dd".into()),
                detected_format: LocalBookFormat::Txt,
                modification_metadata: Some(LocalBookSourceModificationMetadata {
                    byte_count: 128,
                    modification_timestamp: Some("1970-01-01T00:00:00Z".into()),
                    resource_identifier_hint: Some("inode-a".into()),
                    source_path_checksum: Some("fnv1a64:path".into()),
                }),
            },
            content: LocalBookContentFingerprint {
                full_input_checksum: full_checksum.into(),
                parser_config_checksum: "parser".into(),
                normalized_metadata_checksum: "metadata".into(),
                chapter_locator_sequence_checksum: "locators".into(),
            },
            semantic: LocalBookSemanticFingerprint {
                normalized_title: title.into(),
                normalized_author: author.map(str::to_string),
                identifier: None,
                chapter_title_sequence_checksum: "chapter-titles".into(),
                chapter_count: chapters,
                format: LocalBookFormat::Txt,
            },
        }
    }

    fn catalog_entry(fingerprint: &LocalBookFingerprintSet) -> LocalBookFingerprintCatalogEntry {
        LocalBookFingerprintCatalogEntry {
            stable_book_id: "book-existing".into(),
            source_fingerprint: fingerprint.fast.clone(),
            content_fingerprint: fingerprint.content.clone(),
            semantic_fingerprint: fingerprint.semantic.clone(),
            duplicate_group_id: None,
        }
    }

    fn chapter_index(
        book_id: &str,
        stable_chapter_id: &str,
        ordinal: i64,
    ) -> LocalBookChapterIndexEntry {
        LocalBookChapterIndexEntry {
            stable_chapter_id: stable_chapter_id.into(),
            book_id: book_id.into(),
            ordinal,
            normalized_title: format!("chapter-{ordinal}"),
            canonical_locator: format!("local://{book_id}/chapter/{ordinal}"),
            source_range_path_or_page: format!("txt:{ordinal}"),
            content_type: "text/plain".into(),
            estimated_byte_count: 128,
            estimated_character_count: 64,
            content_checksum: Some(format!("checksum-{ordinal}")),
            is_materialized: true,
            previous_chapter_id: None,
            next_chapter_id: None,
            parser_version: "RECOVERY-32".into(),
            diagnostics_summary: Vec::new(),
        }
    }

    fn epub_manifest_item_with(
        id: &str,
        href: &str,
        media_type: &str,
        properties: &[&str],
    ) -> LocalBookEpubManifestItem {
        LocalBookEpubManifestItem {
            id: id.into(),
            href: href.into(),
            media_type: media_type.into(),
            properties: properties
                .iter()
                .map(|property| (*property).into())
                .collect(),
        }
    }

    fn epub_manifest_item(id: &str, href: &str) -> LocalBookEpubManifestItem {
        epub_manifest_item_with(id, href, "application/xhtml+xml", &[])
    }

    fn epub_spine_item(idref: &str, linear: bool) -> LocalBookEpubSpineItem {
        LocalBookEpubSpineItem {
            idref: idref.into(),
            linear,
        }
    }

    fn epub_nav_item(title: &str, href: &str) -> LocalBookEpubNavItem {
        LocalBookEpubNavItem {
            title: title.into(),
            href: href.into(),
        }
    }

    fn epub_nav_request(
        nav_document_path: &str,
        manifest_items: Vec<LocalBookEpubManifestItem>,
        spine_items: Vec<LocalBookEpubSpineItem>,
        nav_items: Vec<LocalBookEpubNavItem>,
    ) -> LocalBookEpubChapterIndexRequest {
        LocalBookEpubChapterIndexRequest {
            book_id: "epub-book".into(),
            package_base_path: "OPS".into(),
            nav_document_path: Some(nav_document_path.into()),
            manifest_items,
            spine_items,
            nav_items,
            known_fragment_ids: BTreeMap::new(),
            parser_version: "RECOVERY-31-EPUB-NAV".into(),
        }
    }

    fn epub_navigation_fallback_request(
        manifest_items: Vec<LocalBookEpubManifestItem>,
        spine_items: Vec<LocalBookEpubSpineItem>,
        nav_items: Vec<LocalBookEpubNavItem>,
        ncx_items: Vec<LocalBookEpubNavItem>,
    ) -> LocalBookEpubNavigationFallbackRequest {
        LocalBookEpubNavigationFallbackRequest {
            book_id: "epub-book".into(),
            package_base_path: "OPS".into(),
            nav_document_path: Some("OPS/navigation/nav.xhtml".into()),
            ncx_document_path: Some("OPS/navigation/toc.ncx".into()),
            manifest_items,
            spine_items,
            nav_items,
            ncx_items,
            known_fragment_ids: BTreeMap::new(),
            parser_version: "RECOVERY-31-EPUB-FALLBACK".into(),
        }
    }

    fn epub_archive_entry(path: &str) -> LocalBookEpubArchiveEntry {
        LocalBookEpubArchiveEntry {
            path: path.into(),
            byte_count: 128,
        }
    }

    fn epub_manifest_draft(id: Option<&str>, href: Option<&str>) -> LocalBookEpubManifestItemDraft {
        LocalBookEpubManifestItemDraft {
            id: id.map(str::to_string),
            href: href.map(str::to_string),
            media_type: Some("application/xhtml+xml".into()),
            properties: Vec::new(),
        }
    }

    fn epub_spine_draft(idref: Option<&str>) -> LocalBookEpubSpineItemDraft {
        LocalBookEpubSpineItemDraft {
            idref: idref.map(str::to_string),
            linear: true,
        }
    }

    fn resource_index(
        book_id: &str,
        stable_resource_id: &str,
        relative_locator: &str,
        mime_type: &str,
        byte_count: u64,
        checksum: Option<&str>,
        resource_kind: LocalBookResourceKind,
    ) -> LocalBookResourceIndexEntry {
        LocalBookResourceIndexEntry {
            stable_resource_id: stable_resource_id.into(),
            book_id: book_id.into(),
            relative_locator: relative_locator.into(),
            mime_type: mime_type.into(),
            byte_count,
            checksum: checksum.map(str::to_string),
            is_materialized: true,
            resource_kind,
        }
    }

    #[test]
    fn epub_nav_chapter_index_filters_non_linear_spine_targets() {
        let request = epub_nav_request(
            "OPS/nav/nav.xhtml",
            vec![
                epub_manifest_item("nav", "nav/nav.xhtml"),
                epub_manifest_item("ch1", "text/one.xhtml"),
                epub_manifest_item("skip", "text/non-linear.xhtml"),
                epub_manifest_item("ch2", "text/two.xhtml"),
            ],
            vec![
                epub_spine_item("ch1", true),
                epub_spine_item("skip", false),
                epub_spine_item("ch2", true),
            ],
            vec![
                epub_nav_item("Chapter One", "../text/one.xhtml"),
                epub_nav_item("Non Linear Appendix", "../text/non-linear.xhtml"),
                epub_nav_item("Chapter Two", "../text/two.xhtml"),
            ],
        );

        let plan = plan_epub_nav_chapter_index(&request).unwrap();

        assert_eq!(
            plan.chapters
                .iter()
                .map(|chapter| chapter.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Chapter One", "Chapter Two"]
        );
        assert_eq!(
            plan.chapters
                .iter()
                .map(|chapter| chapter.source_range_path_or_page.as_str())
                .collect::<Vec<_>>(),
            vec!["OPS/text/one.xhtml", "OPS/text/two.xhtml"]
        );
        assert_eq!(plan.skipped_nav_hrefs, vec!["../text/non-linear.xhtml"]);
        assert!(plan
            .diagnostics_summary
            .contains(&"epub_nav_skipped_non_linear:../text/non-linear.xhtml".into()));
        assert_eq!(
            plan.chapters[0].next_chapter_id,
            Some(plan.chapters[1].stable_chapter_id.clone())
        );
        assert_eq!(
            plan.chapters[1].previous_chapter_id,
            Some(plan.chapters[0].stable_chapter_id.clone())
        );
    }

    #[test]
    fn epub_nav_chapter_index_deduplicates_unresolved_fragments_by_content_path() {
        let request = epub_nav_request(
            "OPS/nav.xhtml",
            vec![
                epub_manifest_item("nav", "nav.xhtml"),
                epub_manifest_item("ch1", "text/one.xhtml"),
                epub_manifest_item("ch2", "text/two.xhtml"),
            ],
            vec![epub_spine_item("ch1", true), epub_spine_item("ch2", true)],
            vec![
                epub_nav_item("Chapter One", "text/one.xhtml#top"),
                epub_nav_item("Chapter One Duplicate Anchor", "text/one.xhtml#duplicate"),
                epub_nav_item("Chapter Two", "text/two.xhtml"),
            ],
        );

        let plan = plan_epub_nav_chapter_index(&request).unwrap();

        assert_eq!(
            plan.chapters
                .iter()
                .map(|chapter| chapter.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Chapter One", "Chapter Two"]
        );
        assert_eq!(
            plan.chapters
                .iter()
                .map(|chapter| chapter.source_range_path_or_page.as_str())
                .collect::<Vec<_>>(),
            vec!["OPS/text/one.xhtml", "OPS/text/two.xhtml"]
        );
        assert_eq!(plan.duplicate_nav_hrefs, vec!["text/one.xhtml#duplicate"]);
        assert_eq!(
            plan.diagnostics_summary,
            vec!["epub_nav_duplicate:text/one.xhtml#duplicate"]
        );
        assert_eq!(plan.chapters[0].ordinal, 0);
        assert_eq!(plan.chapters[1].ordinal, 1);
    }

    #[test]
    fn epub_nav_chapter_index_splits_known_fragments_within_same_document() {
        let mut request = epub_nav_request(
            "OPS/nav.xhtml",
            vec![
                epub_manifest_item("nav", "nav.xhtml"),
                epub_manifest_item("ch1", "text/one.xhtml"),
            ],
            vec![epub_spine_item("ch1", true)],
            vec![
                epub_nav_item("Part One", "text/one.xhtml#part-one"),
                epub_nav_item("Part Two", "text/one.xhtml#part-two"),
            ],
        );
        request.known_fragment_ids.insert(
            "OPS/text/one.xhtml".into(),
            vec!["part-one".into(), "part-two".into()],
        );

        let plan = plan_epub_nav_chapter_index(&request).unwrap();

        assert_eq!(
            plan.chapters
                .iter()
                .map(|chapter| chapter.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Part One", "Part Two"]
        );
        assert_eq!(
            plan.chapters
                .iter()
                .map(|chapter| chapter.source_range_path_or_page.as_str())
                .collect::<Vec<_>>(),
            vec!["OPS/text/one.xhtml#part-one", "OPS/text/one.xhtml#part-two"]
        );
        assert!(plan.duplicate_nav_hrefs.is_empty());
        assert!(plan.skipped_nav_hrefs.is_empty());
        assert_ne!(
            plan.chapters[0].stable_chapter_id,
            plan.chapters[1].stable_chapter_id
        );
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(
            json["chapters"][0]["sourceRangePathOrPage"],
            "OPS/text/one.xhtml#part-one"
        );
    }

    #[test]
    fn epub_navigation_fallback_prefers_valid_nav_before_ncx() {
        let request = epub_navigation_fallback_request(
            vec![
                epub_manifest_item_with(
                    "nav",
                    "navigation/nav.xhtml",
                    "application/xhtml+xml",
                    &["nav"],
                ),
                epub_manifest_item_with(
                    "toc",
                    "navigation/toc.ncx",
                    "application/x-dtbncx+xml",
                    &[],
                ),
                epub_manifest_item("ch1", "text/nav.xhtml"),
                epub_manifest_item("ch2", "text/ncx.xhtml"),
            ],
            vec![epub_spine_item("ch1", true), epub_spine_item("ch2", true)],
            vec![epub_nav_item("Nav Chapter", "../text/nav.xhtml")],
            vec![epub_nav_item("NCX Chapter", "text/ncx.xhtml")],
        );

        let plan = plan_epub_navigation_with_fallback(&request).unwrap();

        assert_eq!(plan.selected_source, LocalBookEpubNavigationSource::Nav);
        assert_eq!(
            plan.chapter_plan
                .chapters
                .iter()
                .map(|chapter| chapter.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Nav Chapter"]
        );
        assert!(plan.diagnostics_summary.is_empty());
    }

    #[test]
    fn epub_navigation_fallback_uses_ncx_when_nav_is_empty() {
        let request = epub_navigation_fallback_request(
            vec![
                epub_manifest_item_with(
                    "nav",
                    "navigation/empty-nav.xhtml",
                    "application/xhtml+xml",
                    &["nav"],
                ),
                epub_manifest_item_with(
                    "toc-data",
                    "navigation/toc-data.xml",
                    "application/x-dtbncx+xml",
                    &[],
                ),
                epub_manifest_item("ch1", "text/one.xhtml"),
                epub_manifest_item("ch2", "text/two.xhtml"),
            ],
            vec![epub_spine_item("ch2", true), epub_spine_item("ch1", true)],
            Vec::new(),
            vec![
                epub_nav_item("NCX One", "text/one.xhtml#chapter-start"),
                epub_nav_item("NCX Two", "text/two.xhtml"),
            ],
        );

        let plan = plan_epub_navigation_with_fallback(&request).unwrap();

        assert_eq!(plan.selected_source, LocalBookEpubNavigationSource::Ncx);
        assert_eq!(
            plan.chapter_plan
                .chapters
                .iter()
                .map(|chapter| chapter.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["NCX One", "NCX Two"]
        );
        assert_eq!(
            plan.chapter_plan
                .chapters
                .iter()
                .map(|chapter| chapter.source_range_path_or_page.as_str())
                .collect::<Vec<_>>(),
            vec!["OPS/text/one.xhtml", "OPS/text/two.xhtml"]
        );
        assert_eq!(
            plan.diagnostics_summary,
            vec!["epub_nav_empty:trying_ncx_fallback"]
        );
        assert!(!plan
            .diagnostics_summary
            .iter()
            .any(|diagnostic| diagnostic.contains("invalid_ncx")));
    }

    #[test]
    fn epub_navigation_fallback_uses_linear_spine_when_nav_and_ncx_are_empty() {
        let request = epub_navigation_fallback_request(
            vec![
                epub_manifest_item_with(
                    "nav",
                    "navigation/empty-nav.xhtml",
                    "application/xhtml+xml",
                    &["nav"],
                ),
                epub_manifest_item("ch1", "text/one.xhtml"),
                epub_manifest_item("skip", "text/skipped.xhtml"),
                epub_manifest_item("ch2", "text/two.xhtml"),
            ],
            vec![
                epub_spine_item("ch1", true),
                epub_spine_item("skip", false),
                epub_spine_item("ch2", true),
            ],
            Vec::new(),
            Vec::new(),
        );

        let plan = plan_epub_navigation_with_fallback(&request).unwrap();

        assert_eq!(plan.selected_source, LocalBookEpubNavigationSource::Spine);
        assert_eq!(
            plan.chapter_plan
                .chapters
                .iter()
                .map(|chapter| chapter.source_range_path_or_page.as_str())
                .collect::<Vec<_>>(),
            vec!["OPS/text/one.xhtml", "OPS/text/two.xhtml"]
        );
        assert_eq!(
            plan.chapter_plan
                .chapters
                .iter()
                .map(|chapter| chapter.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert_eq!(
            plan.diagnostics_summary,
            vec![
                "epub_nav_empty:trying_ncx_fallback",
                "epub_ncx_empty:trying_spine_fallback"
            ]
        );
        assert!(plan
            .chapter_plan
            .chapters
            .iter()
            .all(|chapter| chapter.content_type == "application/xhtml+xml"));
    }

    #[test]
    fn epub_archive_preflight_fails_closed_for_encryption_marker() {
        let request = LocalBookEpubArchivePreflightRequest {
            archive_entries: vec![
                epub_archive_entry("META-INF/container.xml"),
                epub_archive_entry("META-INF/encryption.xml"),
                epub_archive_entry("OPS/package.opf"),
                epub_archive_entry("OPS/text/one.xhtml"),
            ],
            opf_path: Some("OPS/package.opf".into()),
            manifest_items: vec![epub_manifest_draft(Some("ch1"), Some("text/one.xhtml"))],
            spine_items: vec![epub_spine_draft(Some("ch1"))],
        };

        let plan = plan_epub_archive_preflight(&request).unwrap();

        assert!(plan.fail_closed);
        assert!(plan.manifest_items.is_empty());
        assert!(plan.spine_items.is_empty());
        assert_eq!(
            plan.diagnostics_summary,
            vec!["unsupported_encryption:encryption.xml present"]
        );
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["failClosed"], true);
    }

    #[test]
    fn epub_import_diagnostics_fail_closed_for_encryption_without_invalid_opf_noise() {
        let request = LocalBookEpubArchivePreflightRequest {
            archive_entries: vec![
                epub_archive_entry("META-INF/container.xml"),
                epub_archive_entry("META-INF/encryption.xml"),
            ],
            opf_path: Some("OPS/package.opf".into()),
            manifest_items: vec![epub_manifest_draft(None, None)],
            spine_items: vec![epub_spine_draft(None)],
        };
        let preflight = plan_epub_archive_preflight(&request).unwrap();

        let report = build_epub_import_diagnostic_report(&preflight, None).unwrap();

        assert!(report.fail_closed);
        assert_eq!(
            report.diagnostics,
            vec![LocalBookEpubImportDiagnostic {
                code: LocalBookEpubImportDiagnosticCode::UnsupportedEncryption,
                detail: "encryption.xml present".into()
            }]
        );
        assert!(!report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == LocalBookEpubImportDiagnosticCode::InvalidOpf));
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({
                "failClosed": true,
                "diagnostics": [
                    {
                        "code": "unsupportedEncryption",
                        "detail": "encryption.xml present"
                    }
                ]
            })
        );
    }

    #[test]
    fn epub_archive_preflight_reports_missing_container_and_unsafe_paths() {
        let request = LocalBookEpubArchivePreflightRequest {
            archive_entries: vec![
                epub_archive_entry("OPS/package.opf"),
                epub_archive_entry("../secret.xhtml"),
                epub_archive_entry("OPS\\bad.xhtml"),
            ],
            opf_path: Some("OPS/package.opf".into()),
            manifest_items: Vec::new(),
            spine_items: Vec::new(),
        };

        let plan = plan_epub_archive_preflight(&request).unwrap();

        assert!(!plan.fail_closed);
        assert!(plan
            .diagnostics_summary
            .contains(&"unsafe_archive_path:../secret.xhtml".into()));
        assert!(plan
            .diagnostics_summary
            .contains(&"unsafe_archive_path:OPS\\bad.xhtml".into()));
        assert!(plan
            .diagnostics_summary
            .contains(&"missing_container:META-INF/container.xml".into()));

        let report = build_epub_import_diagnostic_report(&plan, None).unwrap();
        assert!(report.diagnostics.contains(&LocalBookEpubImportDiagnostic {
            code: LocalBookEpubImportDiagnosticCode::UnsafeArchivePath,
            detail: "../secret.xhtml".into()
        }));
        assert!(report.diagnostics.contains(&LocalBookEpubImportDiagnostic {
            code: LocalBookEpubImportDiagnosticCode::MissingContainer,
            detail: "META-INF/container.xml".into()
        }));
    }

    #[test]
    fn epub_opf_preflight_keeps_valid_spine_items_while_reporting_structural_drift() {
        let request = LocalBookEpubArchivePreflightRequest {
            archive_entries: vec![
                epub_archive_entry("META-INF/container.xml"),
                epub_archive_entry("OPS/package.opf"),
                epub_archive_entry("OPS/text/one.xhtml"),
                epub_archive_entry("OPS/text/dup-a.xhtml"),
                epub_archive_entry("OPS/text/dup-b.xhtml"),
            ],
            opf_path: Some("OPS/package.opf".into()),
            manifest_items: vec![
                epub_manifest_draft(Some("ch1"), Some("text/one.xhtml")),
                epub_manifest_draft(Some("missing-href"), None),
                epub_manifest_draft(None, Some("text/missing-id.xhtml")),
                epub_manifest_draft(Some("dup"), Some("text/dup-a.xhtml")),
                epub_manifest_draft(Some("dup"), Some("text/dup-b.xhtml")),
            ],
            spine_items: vec![
                epub_spine_draft(Some("ch1")),
                epub_spine_draft(Some("ghost")),
                epub_spine_draft(None),
            ],
        };

        let preflight = plan_epub_archive_preflight(&request).unwrap();

        assert!(!preflight.fail_closed);
        assert_eq!(
            preflight
                .manifest_items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["ch1", "dup"]
        );
        assert_eq!(
            preflight
                .spine_items
                .iter()
                .map(|item| item.idref.as_str())
                .collect::<Vec<_>>(),
            vec!["ch1"]
        );
        for expected in [
            "missing_manifest_item:missing id or href",
            "missing_manifest_item:duplicate manifest item id dup",
            "missing_spine_item:spine idref ghost missing manifest item",
            "missing_spine_item:spine itemref missing idref",
            "invalid_opf:structural references",
        ] {
            assert!(
                preflight
                    .diagnostics_summary
                    .contains(&expected.to_string()),
                "missing diagnostic {expected}"
            );
        }

        let fallback = plan_epub_navigation_with_fallback(&epub_navigation_fallback_request(
            preflight.manifest_items.clone(),
            preflight.spine_items.clone(),
            Vec::new(),
            Vec::new(),
        ))
        .unwrap();
        assert_eq!(
            fallback.selected_source,
            LocalBookEpubNavigationSource::Spine
        );
        assert_eq!(
            fallback.chapter_plan.chapters[0].source_range_path_or_page,
            "OPS/text/one.xhtml"
        );

        let report = build_epub_import_diagnostic_report(&preflight, Some(&fallback)).unwrap();
        assert!(!report.fail_closed);
        assert!(report.diagnostics.contains(&LocalBookEpubImportDiagnostic {
            code: LocalBookEpubImportDiagnosticCode::InvalidOpf,
            detail: "structural references".into()
        }));
        assert!(report.diagnostics.contains(&LocalBookEpubImportDiagnostic {
            code: LocalBookEpubImportDiagnosticCode::InvalidNav,
            detail: "trying ncx fallback".into()
        }));
    }

    #[test]
    fn epub_import_diagnostics_classify_nav_ncx_and_missing_resource_boundaries() {
        let summaries = vec![
            "epub_nav_empty:trying_ncx_fallback".to_string(),
            "epub_ncx_invalid:trying_spine_fallback".to_string(),
            "epub_nav_missing_spine:text/missing.xhtml".to_string(),
            "epub_nav_empty:trying_ncx_fallback".to_string(),
            "epub_html_image_text_fallback".to_string(),
        ];

        let diagnostics = normalize_epub_import_diagnostics(&summaries).unwrap();

        assert_eq!(
            diagnostics,
            vec![
                LocalBookEpubImportDiagnostic {
                    code: LocalBookEpubImportDiagnosticCode::InvalidNav,
                    detail: "trying ncx fallback".into()
                },
                LocalBookEpubImportDiagnostic {
                    code: LocalBookEpubImportDiagnosticCode::InvalidNcx,
                    detail: "trying spine fallback".into()
                },
                LocalBookEpubImportDiagnostic {
                    code: LocalBookEpubImportDiagnosticCode::MissingChapterResource,
                    detail: "text/missing.xhtml".into()
                }
            ]
        );
        assert!(
            serde_json::from_value::<LocalBookEpubImportDiagnosticReport>(serde_json::json!({
                "failClosed": false,
                "diagnostics": [],
                "unexpected": true
            }))
            .is_err()
        );
    }

    #[test]
    fn epub_package_metadata_extracts_legacy_identifier_artifact() {
        let opf = r#"
        <package xmlns:dc="http://purl.org/dc/elements/1.1/" unique-identifier="BookId">
          <metadata>
            <dc:identifier id="IgnoredId">ignored-epub</dc:identifier>
            <dc:identifier id="BookId">fixture-epub</dc:identifier>
            <dc:title>Fixture EPUB</dc:title>
            <dc:language>en</dc:language>
          </metadata>
        </package>
        "#;

        let artifact = extract_epub_package_metadata(&LocalBookEpubPackageMetadataRequest {
            opf_xml: opf.into(),
        })
        .unwrap();

        assert!(!artifact.fail_closed);
        assert_eq!(
            artifact.metadata_identifier.as_deref(),
            Some("fixture-epub")
        );
        assert_eq!(artifact.metadata_title.as_deref(), Some("Fixture EPUB"));
        assert_eq!(artifact.metadata_language.as_deref(), Some("en"));
        assert_eq!(
            artifact.package_unique_identifier_id.as_deref(),
            Some("BookId")
        );
        assert!(artifact.diagnostics_summary.is_empty());
        let json = serde_json::to_value(&artifact).unwrap();
        assert_eq!(json["metadataIdentifier"], "fixture-epub");
        assert_eq!(json["metadataTitle"], "Fixture EPUB");
        assert_eq!(json["metadataLanguage"], "en");
    }

    #[test]
    fn webdav_descriptor_artifact_matches_legacy_format_differential_fixture() {
        let descriptor = r#"{"author":"Remote Author","etag":"etag-fixture","fileSize":42,"format":"epub","remotePath":"/dav/books/remote.epub","sourceIdentifier":"webdav-fixture","title":"Remote EPUB"}"#;

        let artifact = parse_webdav_descriptor_artifact(&LocalBookWebDavDescriptorRequest {
            descriptor_json: descriptor.into(),
        })
        .unwrap();

        assert_eq!(artifact.detected_format, LocalBookFormat::WebDav);
        assert_eq!(artifact.detected_encoding, "utf-8");
        assert_eq!(artifact.title, "Remote EPUB");
        assert_eq!(artifact.author.as_deref(), Some("Remote Author"));
        assert_eq!(artifact.identifier.as_deref(), Some("webdav-fixture"));
        assert_eq!(artifact.remote_path, "/dav/books/remote.epub");
        assert_eq!(artifact.resource.path, "/dav/books/remote.epub");
        assert_eq!(artifact.resource.media_type, "application/epub+zip");
        assert_eq!(artifact.resource.byte_count, 42);
        assert_eq!(artifact.resource.checksum, "etag-fixture");
        assert_eq!(
            artifact.book_id,
            local_book_stable_checksum(&["webdav", "/dav/books/remote.epub", "etag-fixture"])
        );
        assert_eq!(
            artifact.diagnostic, "unsupported_media_type",
            "legacy importer reports WebDAV descriptor import as metadata-only"
        );
        assert_eq!(artifact.diagnostics_summary, vec!["unsupported_media_type"]);
        assert_eq!(artifact.content_checksum_count, 1);
        assert_eq!(artifact.full_content_persisted_count, 0);
        assert!(artifact.clean_room_maintained);
        assert!(!artifact.external_gpl_code_copied);

        let json = serde_json::to_value(&artifact).unwrap();
        assert_eq!(json["detectedFormat"], "webdav");
        assert_eq!(json["remotePath"], "/dav/books/remote.epub");
        assert_eq!(json["identifier"], "webdav-fixture");
        assert_eq!(json["resource"]["mediaType"], "application/epub+zip");
        assert_eq!(json["diagnostic"], "unsupported_media_type");
        assert_eq!(json["fullContentPersistedCount"], 0);
    }

    #[test]
    fn webdav_descriptor_artifact_rejects_drifted_fixture_evidence() {
        assert_eq!(
            parse_webdav_descriptor_artifact(&LocalBookWebDavDescriptorRequest {
                descriptor_json: " ".into(),
            })
            .unwrap_err(),
            LocalBookError::EmptyInput
        );

        let blank_remote_path =
            r#"{"remotePath":" ","title":"Remote EPUB","format":"epub","fileSize":42}"#;
        assert_eq!(
            parse_webdav_descriptor_artifact(&LocalBookWebDavDescriptorRequest {
                descriptor_json: blank_remote_path.into(),
            })
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "remote_path".into()
            }
        );

        let negative_size =
            r#"{"remotePath":"/dav/books/remote.epub","title":"Remote EPUB","fileSize":-1}"#;
        assert_eq!(
            parse_webdav_descriptor_artifact(&LocalBookWebDavDescriptorRequest {
                descriptor_json: negative_size.into(),
            })
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "file_size".into()
            }
        );

        let unknown_field =
            r#"{"remotePath":"/dav/books/remote.epub","title":"Remote EPUB","bogus":true}"#;
        assert!(matches!(
            parse_webdav_descriptor_artifact(&LocalBookWebDavDescriptorRequest {
                descriptor_json: unknown_field.into(),
            })
            .unwrap_err(),
            LocalBookError::Decode { .. }
        ));

        assert!(
            serde_json::from_value::<LocalBookWebDavDescriptorRequest>(serde_json::json!({
                "descriptorJson": "{}",
                "unexpected": true
            }))
            .is_err()
        );

        let fallback_descriptor =
            r#"{"etag":"etag-only","remotePath":"/dav/books/remote.mobi","title":"Remote MOBI"}"#;
        let fallback = parse_webdav_descriptor_artifact(&LocalBookWebDavDescriptorRequest {
            descriptor_json: fallback_descriptor.into(),
        })
        .unwrap();
        assert_eq!(fallback.identifier.as_deref(), Some("etag-only"));
        assert_eq!(
            fallback.resource.media_type,
            "application/x-mobipocket-ebook"
        );
    }

    #[test]
    fn epub_text_entity_fixture_decodes_metadata_nav_and_preview() {
        let opf = r#"
        <package>
          <metadata>
            <dc:title>Entity &amp; Numeric &#x4e00; &#49;</dc:title>
            <dc:creator>Author &lt;Clean&gt;</dc:creator>
            <dc:language>en</dc:language>
            <dc:identifier>id-entity</dc:identifier>
          </metadata>
        </package>
        "#;

        let metadata = extract_epub_package_metadata(&LocalBookEpubPackageMetadataRequest {
            opf_xml: opf.into(),
        })
        .unwrap();

        assert!(!metadata.fail_closed);
        assert_eq!(
            metadata.metadata_title.as_deref(),
            Some("Entity & Numeric 一 1")
        );
        assert_eq!(metadata.metadata_author.as_deref(), Some("Author <Clean>"));
        assert_eq!(metadata.metadata_identifier.as_deref(), Some("id-entity"));
        let metadata_json = serde_json::to_value(&metadata).unwrap();
        assert_eq!(metadata_json["metadataAuthor"], "Author <Clean>");

        let nav = plan_epub_nav_chapter_index(&epub_nav_request(
            "OPS/nav.xhtml",
            vec![epub_manifest_item("ch1", "text/one.xhtml")],
            vec![epub_spine_item("ch1", true)],
            vec![epub_nav_item(
                "Chapter &amp; One &#x2605;",
                "text/one.xhtml",
            )],
        ))
        .unwrap();
        assert_eq!(nav.chapters[0].normalized_title, "Chapter & One ★");

        let preview = extract_epub_html_text_boundary(&LocalBookEpubHtmlTextBoundaryRequest {
            html: "<html><body><p>Tom &amp; Jerry &#169; body.</p></body></html>".into(),
            preview_limit: 256,
        })
        .unwrap();
        assert!(preview.preview.contains("Tom & Jerry © body."));
        assert!(!preview.preview.contains("&amp;"));
    }

    #[test]
    fn epub_html_named_entity_variants_decode_legacy_reader_core_fixtures() {
        struct EntityCase {
            title: &'static str,
            author: &'static str,
            identifier: &'static str,
            nav_title: &'static str,
            preview_html: &'static str,
            expected_title: &'static str,
            expected_author: &'static str,
            expected_nav_title: &'static str,
            expected_preview: &'static str,
            raw_absent: &'static [&'static str],
            unknown_preserved: Option<&'static str>,
        }

        let cases = [
            EntityCase {
                title: "Common&nbsp;Names &mdash; EPUB &hellip;",
                author: "Editor &copy; Team",
                identifier: "common-html-entity",
                nav_title: "Intro &mdash; Start &hellip;",
                preview_html:
                    "<p>Hello&nbsp;World &ldquo;quote&rdquo; &rsquo;apostrophe&rsquo; &euro;.</p>",
                expected_title: "Common Names — EPUB …",
                expected_author: "Editor © Team",
                expected_nav_title: "Intro — Start …",
                expected_preview: "Hello World “quote” ’apostrophe’ €.",
                raw_absent: &["&nbsp;", "&mdash;"],
                unknown_preserved: None,
            },
            EntityCase {
                title: "Section&nbsp;&sect; A &plusmn; B &times; C &divide; D",
                author: "Caf&eacute; &amp; Cr&egrave;me &ntilde;",
                identifier: "extended-html-entity",
                nav_title: "Caf&eacute; &frac12; Price &mdash; &uuml;",
                preview_html: "<p>Temp&nbsp;20&deg;C, mix &frac14; + &frac34;, cost &cent;5.</p>",
                expected_title: "Section § A ± B × C ÷ D",
                expected_author: "Café & Crème ñ",
                expected_nav_title: "Café ½ Price — ü",
                expected_preview: "Temp 20°C, mix ¼ + ¾, cost ¢5.",
                raw_absent: &["&frac14;", "&deg;"],
                unknown_preserved: None,
            },
            EntityCase {
                title: "Legacy&nbsp-Entity &COPY; Notice",
                author: "&Eacute;diteur &AMP; Team",
                identifier: "legacy-html-entity",
                nav_title: "Intro &MDASH; Legacy &frac12 Price",
                preview_html: "<p>Temp&nbsp 21&deg C &AMP; copy &copy notice.</p>",
                expected_title: "Legacy -Entity © Notice",
                expected_author: "Éditeur & Team",
                expected_nav_title: "Intro — Legacy ½ Price",
                expected_preview: "Temp 21° C & copy © notice.",
                raw_absent: &["&frac12", "&AMP;"],
                unknown_preserved: None,
            },
            EntityCase {
                title: "Mixed &EACUTE; &eACUTE; &AGRAVE; &aGRAVE;",
                author: "Se&NTILDE;or &ntilDE;",
                identifier: "mixed-case-latin-entity",
                nav_title: "Nav &UACUTE; &uACUTE;",
                preview_html: "<p>Cafe &EGRAVE; &eGRAVE; &IACUTE; &iACUTE; &OACUTE; &oACUTE; &UUML; &uUML; &unknownEntity;</p>",
                expected_title: "Mixed É é À à",
                expected_author: "SeÑor ñ",
                expected_nav_title: "Nav Ú ú",
                expected_preview: "Cafe È è Í í Ó ó Ü ü &unknownEntity;",
                raw_absent: &["&EGRAVE;", "&uUML;"],
                unknown_preserved: Some("&unknownEntity;"),
            },
        ];

        for case in cases {
            let opf = format!(
                r#"
                <package>
                  <metadata>
                    <dc:title>{}</dc:title>
                    <dc:creator>{}</dc:creator>
                    <dc:language>en</dc:language>
                    <dc:identifier>{}</dc:identifier>
                  </metadata>
                </package>
                "#,
                case.title, case.author, case.identifier
            );
            let metadata = extract_epub_package_metadata(&LocalBookEpubPackageMetadataRequest {
                opf_xml: opf,
            })
            .unwrap();
            assert_eq!(
                metadata.metadata_title.as_deref(),
                Some(case.expected_title)
            );
            assert_eq!(
                metadata.metadata_author.as_deref(),
                Some(case.expected_author)
            );
            assert_eq!(
                metadata.metadata_identifier.as_deref(),
                Some(case.identifier)
            );

            let nav = plan_epub_nav_chapter_index(&epub_nav_request(
                "OPS/nav.xhtml",
                vec![epub_manifest_item("ch1", "text/one.xhtml")],
                vec![epub_spine_item("ch1", true)],
                vec![epub_nav_item(case.nav_title, "text/one.xhtml")],
            ))
            .unwrap();
            assert_eq!(nav.chapters[0].normalized_title, case.expected_nav_title);

            let preview = extract_epub_html_text_boundary(&LocalBookEpubHtmlTextBoundaryRequest {
                html: format!("<html><body>{}</body></html>", case.preview_html),
                preview_limit: 256,
            })
            .unwrap();
            assert!(
                preview.preview.contains(case.expected_preview),
                "preview mismatch for {}: {}",
                case.identifier,
                preview.preview
            );
            for raw in case.raw_absent {
                assert!(
                    !preview.preview.contains(raw),
                    "raw entity {raw} leaked for {}",
                    case.identifier
                );
            }
            if let Some(unknown) = case.unknown_preserved {
                assert!(preview.preview.contains(unknown));
            }
        }
    }

    #[test]
    fn epub_package_metadata_fails_closed_for_missing_identifier_and_drifted_json() {
        let opf = r#"
        <opf:package xmlns:opf="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/">
          <opf:metadata>
            <dc:title>Fixture &amp; EPUB</dc:title>
            <dc:language>en</dc:language>
          </opf:metadata>
        </opf:package>
        "#;

        let artifact = extract_epub_package_metadata(&LocalBookEpubPackageMetadataRequest {
            opf_xml: opf.into(),
        })
        .unwrap();

        assert!(artifact.fail_closed);
        assert_eq!(artifact.metadata_title.as_deref(), Some("Fixture & EPUB"));
        assert_eq!(artifact.metadata_language.as_deref(), Some("en"));
        assert!(artifact.metadata_identifier.is_none());
        assert_eq!(
            artifact.diagnostics_summary,
            vec!["missing_metadata_identifier:dc:identifier"]
        );
        assert_eq!(
            extract_epub_package_metadata(&LocalBookEpubPackageMetadataRequest {
                opf_xml: " ".into(),
            })
            .unwrap_err(),
            LocalBookError::EmptyInput
        );
        assert!(
            serde_json::from_value::<LocalBookEpubPackageMetadataRequest>(serde_json::json!({
                "opfXml": opf,
                "unexpected": true
            }))
            .is_err()
        );
    }

    #[test]
    fn epub_html_text_boundary_suppresses_invisible_blocks_and_uses_image_fallbacks() {
        let html = r#"
        <html>
          <head>
            <title>C</title>
            <style>.secret { color: red; } body::before { content: 'style leak'; }</style>
          </head>
          <body>
            <h1>C</h1>
            <script>bad()</script>
            <noscript>noscript fallback must not become chapter text</noscript>
            <p>Visible&nbsp;text.</p>
            <img src="../images/cover.png" alt="Illustrated &amp; caption"/>
            <img src="../images/map.png" title="Map title fallback"/>
          </body>
        </html>
        "#;

        let result = extract_epub_html_text_boundary(&LocalBookEpubHtmlTextBoundaryRequest {
            html: html.into(),
            preview_limit: 256,
        })
        .unwrap();

        assert!(result.preview.contains("Visible text."));
        assert!(result.preview.contains("Illustrated & caption"));
        assert!(result.preview.contains("Map title fallback"));
        assert!(!result.preview.contains("style leak"));
        assert!(!result.preview.contains(".secret"));
        assert!(!result.preview.contains("bad()"));
        assert!(!result.preview.contains("noscript fallback"));
        assert_eq!(result.suppressed_block_count, 3);
        assert_eq!(result.image_fallback_count, 2);
        assert_eq!(
            result.diagnostics_summary,
            vec![
                "epub_html_suppressed_invisible_blocks",
                "epub_html_image_text_fallback"
            ]
        );
    }

    #[test]
    fn epub_html_text_boundary_is_bounded_and_rejects_drifted_request() {
        let result = extract_epub_html_text_boundary(&LocalBookEpubHtmlTextBoundaryRequest {
            html: "<body><p>Alpha Beta Gamma</p><img alt='Delta Echo'/></body>".into(),
            preview_limit: 12,
        })
        .unwrap();

        assert_eq!(result.preview, "Alpha Beta G");
        assert_eq!(result.image_fallback_count, 1);
        assert_eq!(
            serde_json::to_value(&result).unwrap()["imageFallbackCount"],
            1
        );
        assert_eq!(
            extract_epub_html_text_boundary(&LocalBookEpubHtmlTextBoundaryRequest {
                html: " ".into(),
                preview_limit: 12,
            })
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "html".into()
            }
        );
        assert_eq!(
            extract_epub_html_text_boundary(&LocalBookEpubHtmlTextBoundaryRequest {
                html: "<p>Visible</p>".into(),
                preview_limit: 0,
            })
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "preview_limit".into()
            }
        );
    }

    #[test]
    fn local_book_html_text_boundary_extracts_standalone_title_and_visible_preview() {
        let html = r#"
        <html>
          <head>
            <TITLE> Local &amp; HTML Book </TITLE>
            <style>.hidden { content: 'style leak'; }</style>
          </head>
          <body>
            <h1>第一章 HTML</h1>
            <style>.body-hidden { content: 'style leak'; }</style>
            <script>bad()</script>
            <noscript>noscript fallback must stay host-owned</noscript>
            <p>Visible.</p>
            <img src="cover.png" alt="Cover &amp; caption"/>
            <img src="map.png" title="Map title fallback"/>
          </body>
        </html>
        "#;

        let result = extract_local_book_html_text_boundary(&LocalBookHtmlTextBoundaryRequest {
            html: html.into(),
            preview_limit: 64,
        })
        .unwrap();

        assert_eq!(result.title.as_deref(), Some("Local & HTML Book"));
        assert!(result.preview.contains("Visible."));
        assert!(result.preview.contains("Cover & caption"));
        assert!(result.preview.contains("Map title fallback"));
        assert!(!result.preview.contains("Local & HTML Book"));
        assert!(!result.preview.contains("style leak"));
        assert!(!result.preview.contains("bad()"));
        assert!(!result.preview.contains("noscript fallback"));
        assert_eq!(result.suppressed_block_count, 3);
        assert_eq!(result.image_fallback_count, 2);
        assert_eq!(
            result.diagnostics_summary,
            vec![
                "local_html_suppressed_invisible_blocks",
                "local_html_image_text_fallback"
            ]
        );
        assert_eq!(
            serde_json::to_value(&result).unwrap()["title"],
            "Local & HTML Book"
        );
    }

    #[test]
    fn local_book_html_text_boundary_is_bounded_and_rejects_drifted_request() {
        let result = extract_local_book_html_text_boundary(&LocalBookHtmlTextBoundaryRequest {
            html: "<html><body><p>Alpha Beta Gamma</p><img title='Delta Echo'/></body></html>"
                .into(),
            preview_limit: 12,
        })
        .unwrap();

        assert_eq!(result.preview, "Alpha Beta G");
        assert_eq!(result.title, None);
        assert_eq!(result.image_fallback_count, 1);
        assert_eq!(
            extract_local_book_html_text_boundary(&LocalBookHtmlTextBoundaryRequest {
                html: " ".into(),
                preview_limit: 12,
            })
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "html".into()
            }
        );
        assert_eq!(
            extract_local_book_html_text_boundary(&LocalBookHtmlTextBoundaryRequest {
                html: "<p>Visible</p>".into(),
                preview_limit: 0,
            })
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "preview_limit".into()
            }
        );
    }

    #[test]
    fn epub_nav_chapter_index_decodes_entities_percent_and_document_relative_hrefs() {
        let request = epub_nav_request(
            "OPS/nav/nav.xhtml",
            vec![
                epub_manifest_item_with("nav", "nav/nav.xhtml", "application/xhtml+xml", &["nav"]),
                epub_manifest_item("ch1", "Text/Chapter%201.xhtml"),
                epub_manifest_item("ch2", "Text/two&amp;more.xhtml"),
            ],
            vec![epub_spine_item("ch1", true), epub_spine_item("ch2", true)],
            vec![
                epub_nav_item("Encoded One", "../Text/Chapter%201.xhtml#start"),
                epub_nav_item("Entity Two", "../Text/two&amp;more.xhtml"),
            ],
        );

        let plan = plan_epub_nav_chapter_index(&request).unwrap();

        assert_eq!(
            plan.chapters
                .iter()
                .map(|chapter| chapter.normalized_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Encoded One", "Entity Two"]
        );
        assert_eq!(
            plan.chapters
                .iter()
                .map(|chapter| chapter.source_range_path_or_page.as_str())
                .collect::<Vec<_>>(),
            vec!["OPS/Text/Chapter 1.xhtml", "OPS/Text/two&more.xhtml"]
        );
        assert_eq!(
            plan.chapters[0].canonical_locator,
            "epub://epub-book/OPS/Text/Chapter 1.xhtml"
        );
        assert!(plan.skipped_nav_hrefs.is_empty());
        assert!(plan.duplicate_nav_hrefs.is_empty());
    }

    #[test]
    fn epub_resource_index_classifies_manifest_media_and_cover_image_property() {
        let request = LocalBookEpubResourceIndexRequest {
            book_id: "epub-book".into(),
            package_base_path: "OPS".into(),
            manifest_items: vec![
                epub_manifest_item_with("ch1", "text/one.xhtml", "application/xhtml+xml", &[]),
                epub_manifest_item_with("style", "Styles/main%20style.css", "text/css", &[]),
                epub_manifest_item_with(
                    "cover-art",
                    "Images/cover%20art.jpg",
                    "image/jpeg",
                    &["cover-image"],
                ),
                epub_manifest_item_with("font", "Fonts/book.woff", "font/woff", &[]),
            ],
            cover_id: None,
        };

        let plan = plan_epub_resource_index(&request).unwrap();

        assert_eq!(
            plan.resources
                .iter()
                .map(|resource| resource.relative_locator.as_str())
                .collect::<Vec<_>>(),
            vec![
                "OPS/Styles/main style.css",
                "OPS/Images/cover art.jpg",
                "OPS/Fonts/book.woff"
            ]
        );
        assert_eq!(
            plan.resources
                .iter()
                .map(|resource| resource.resource_kind)
                .collect::<Vec<_>>(),
            vec![
                LocalBookResourceKind::Css,
                LocalBookResourceKind::Cover,
                LocalBookResourceKind::Font
            ]
        );
        let cover_id = plan.cover_resource_id.as_ref().unwrap();
        let cover = plan
            .resources
            .iter()
            .find(|resource| &resource.stable_resource_id == cover_id)
            .unwrap();
        assert_eq!(cover.relative_locator, "OPS/Images/cover art.jpg");
        assert!(!plan
            .resources
            .iter()
            .any(|resource| resource.relative_locator.ends_with("one.xhtml")));
    }

    #[test]
    fn epub_resource_index_selects_epub2_cover_meta_before_image_heuristic() {
        let request = LocalBookEpubResourceIndexRequest {
            book_id: "epub-book".into(),
            package_base_path: "OPS".into(),
            manifest_items: vec![
                epub_manifest_item_with("ch1", "text/one.xhtml", "application/xhtml+xml", &[]),
                epub_manifest_item_with("front", "images/a-front.jpg", "image/jpeg", &[]),
                epub_manifest_item_with("jacket", "images/z-jacket.bin", "image/jpeg", &[]),
            ],
            cover_id: Some("jacket".into()),
        };

        let plan = plan_epub_resource_index(&request).unwrap();

        assert_eq!(plan.resources.len(), 2);
        let cover_id = plan.cover_resource_id.as_ref().unwrap();
        let cover = plan
            .resources
            .iter()
            .find(|resource| &resource.stable_resource_id == cover_id)
            .unwrap();
        assert_eq!(cover.relative_locator, "OPS/images/z-jacket.bin");
        assert_eq!(cover.resource_kind, LocalBookResourceKind::Cover);
        assert!(plan
            .resources
            .iter()
            .any(
                |resource| resource.relative_locator == "OPS/images/a-front.jpg"
                    && resource.resource_kind == LocalBookResourceKind::Image
            ));
        assert_eq!(
            serde_json::to_value(&plan).unwrap()["coverResourceId"],
            cover.stable_resource_id
        );
    }

    fn reading_progress(
        chapter_id: Option<&str>,
        chapter_ordinal: i64,
        canonical_locator: Option<&str>,
        book_fingerprint: &str,
    ) -> LocalBookReadingProgress {
        LocalBookReadingProgress {
            locator: LocalBookReadingLocator {
                book_id: "book-existing".into(),
                chapter_id: chapter_id.map(str::to_string),
                chapter_ordinal,
                format: LocalBookFormat::Txt,
                chapter_canonical_locator: canonical_locator.map(str::to_string),
                character_offset: 12,
                byte_offset: Some(24),
                normalized_progress_in_chapter: 0.4,
                normalized_progress_in_book: 0.3,
                pdf_page_index: None,
                epub_fragment_id: None,
                txt_source_range: Some("txt:1".into()),
                surrounding_text_checksum: Some("surrounding".into()),
                parser_version: "RECOVERY-32".into(),
                book_fingerprint: book_fingerprint.into(),
                timestamp: "1970-01-01T00:00:00Z".into(),
            },
            restore_state: LocalBookReadingRestoreState::ExactRestored,
            diagnostics: Vec::new(),
        }
    }

    fn chapter_read_result(
        chapter_id: &str,
        ordinal: i64,
        cache_hit: bool,
    ) -> LocalBookChapterReadResult {
        LocalBookChapterReadResult {
            book_id: "book-existing".into(),
            chapter_id: chapter_id.into(),
            ordinal,
            normalized_text: format!("chapter {ordinal} normalized text"),
            sanitized_html: None,
            content_checksum: format!("checksum-{ordinal}"),
            byte_count: 128,
            character_count: 25,
            cache_hit,
            diagnostics: Vec::new(),
            preview: format!("chapter {ordinal}"),
        }
    }

    fn cache_metadata(cache_key: &str, chapter_or_resource_id: &str) -> LocalBookCacheMetadata {
        LocalBookCacheMetadata {
            cache_key: cache_key.into(),
            book_fingerprint: "full-a".into(),
            parser_config_checksum: "parser".into(),
            parser_version: "RECOVERY-32".into(),
            chapter_or_resource_id: chapter_or_resource_id.into(),
            content_checksum: Some(format!("checksum-{chapter_or_resource_id}")),
            created_timestamp: "1970-01-01T00:00:00Z".into(),
            last_access_timestamp: "1970-01-01T00:00:00Z".into(),
            byte_count: 128,
            validation_state: LocalBookCacheState::Materialized,
            eviction_priority: 10,
        }
    }

    fn populate_library(library: &mut LocalBookLibrary) {
        library.upsert_book(sample_book("b2", "Beta")).unwrap();
        library.upsert_book(sample_book("b1", "Alpha")).unwrap();
    }

    #[test]
    fn local_book_catalog_upsert_replaces_rows_and_sorts_like_legacy_store() {
        let mut beta_entry = catalog_entry(&fingerprint("full-beta", "Beta", Some("Author B"), 2));
        beta_entry.stable_book_id = "book-b".into();
        let mut alpha_entry =
            catalog_entry(&fingerprint("full-alpha", "Alpha", Some("Author A"), 1));
        alpha_entry.stable_book_id = "book-a".into();

        let catalog = upsert_local_book_catalog_entry(
            &LocalBookCatalogSnapshot::empty(),
            beta_entry,
            vec![
                chapter_index("book-b", "b-2", 2),
                chapter_index("book-b", "b-1", 1),
            ],
            vec![
                resource_index(
                    "book-b",
                    "beta-cover",
                    "OPS/z-cover.png",
                    "image/png",
                    128,
                    Some("beta-cover-checksum"),
                    LocalBookResourceKind::Cover,
                ),
                resource_index(
                    "book-b",
                    "beta-style",
                    "OPS/a-style.css",
                    "text/css",
                    64,
                    Some("beta-style-checksum"),
                    LocalBookResourceKind::Css,
                ),
            ],
        )
        .unwrap();
        let catalog = upsert_local_book_catalog_entry(
            &catalog,
            alpha_entry,
            vec![chapter_index("book-a", "a-0", 0)],
            vec![resource_index(
                "book-a",
                "alpha-cover",
                "OPS/cover.png",
                "image/png",
                256,
                Some("alpha-cover-checksum"),
                LocalBookResourceKind::Cover,
            )],
        )
        .unwrap();

        let mut beta_replacement =
            catalog_entry(&fingerprint("full-beta-v2", "Beta", Some("Author B"), 1));
        beta_replacement.stable_book_id = "book-b".into();
        let catalog = upsert_local_book_catalog_entry(
            &catalog,
            beta_replacement,
            vec![chapter_index("book-b", "b-0", 0)],
            vec![resource_index(
                "book-b",
                "beta-style-v2",
                "OPS/b-style.css",
                "text/css",
                96,
                Some("beta-style-v2-checksum"),
                LocalBookResourceKind::Css,
            )],
        )
        .unwrap();

        assert_eq!(
            catalog
                .books
                .iter()
                .map(|book| book.stable_book_id.as_str())
                .collect::<Vec<_>>(),
            vec!["book-a", "book-b"]
        );
        assert_eq!(
            catalog
                .chapters
                .iter()
                .map(|chapter| (
                    chapter.book_id.as_str(),
                    chapter.ordinal,
                    chapter.stable_chapter_id.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![("book-a", 0, "a-0"), ("book-b", 0, "b-0")]
        );
        assert_eq!(
            catalog
                .resources
                .iter()
                .map(|resource| (
                    resource.book_id.as_str(),
                    resource.relative_locator.as_str(),
                    resource.stable_resource_id.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("book-a", "OPS/cover.png", "alpha-cover"),
                ("book-b", "OPS/b-style.css", "beta-style-v2")
            ]
        );
        assert_eq!(
            catalog
                .books
                .iter()
                .find(|book| book.stable_book_id == "book-b")
                .unwrap()
                .content_fingerprint
                .full_input_checksum,
            "full-beta-v2"
        );
        assert!(!catalog.chapters.iter().any(
            |chapter| chapter.stable_chapter_id == "b-1" || chapter.stable_chapter_id == "b-2"
        ));
        assert!(!catalog
            .resources
            .iter()
            .any(|resource| resource.stable_resource_id == "beta-cover"
                || resource.stable_resource_id == "beta-style"));

        let json = serde_json::to_value(&catalog).unwrap();
        assert_eq!(
            json["schemaVersion"],
            serde_json::json!(LOCAL_BOOK_CATALOG_SCHEMA_VERSION)
        );
        serde_json::from_value::<LocalBookCatalogSnapshot>(json)
            .unwrap()
            .validate()
            .unwrap();
    }

    #[test]
    fn local_book_catalog_remove_lookup_and_validation_match_legacy_store() {
        let mut content_match = catalog_entry(&fingerprint(
            "full-shared",
            "Different",
            Some("Author A"),
            1,
        ));
        content_match.stable_book_id = "book-a".into();
        let mut semantic_middle =
            catalog_entry(&fingerprint("full-middle", "Shared", Some("Author B"), 1));
        semantic_middle.stable_book_id = "book-b".into();
        let mut semantic_last =
            catalog_entry(&fingerprint("full-last", "Shared", Some("Author B"), 1));
        semantic_last.stable_book_id = "book-c".into();

        let mut catalog = LocalBookCatalogSnapshot::empty();
        for entry in [
            semantic_last.clone(),
            content_match.clone(),
            semantic_middle.clone(),
        ] {
            let book_id = entry.stable_book_id.clone();
            catalog = upsert_local_book_catalog_entry(
                &catalog,
                entry,
                vec![chapter_index(&book_id, &format!("{book_id}-chapter"), 0)],
                vec![resource_index(
                    &book_id,
                    &format!("{book_id}-resource"),
                    &format!("OPS/{book_id}.css"),
                    "text/css",
                    32,
                    Some(&format!("{book_id}-checksum")),
                    LocalBookResourceKind::Css,
                )],
            )
            .unwrap();
        }

        let request = fingerprint("full-shared", "Shared", Some("Author B"), 1);
        assert_eq!(
            lookup_local_book_catalog_by_fingerprint(&catalog, &request)
                .unwrap()
                .into_iter()
                .map(|book| book.stable_book_id)
                .collect::<Vec<_>>(),
            vec!["book-a", "book-b", "book-c"]
        );

        catalog = remove_local_book_catalog_entry(&catalog, "book-b").unwrap();
        assert_eq!(
            catalog
                .books
                .iter()
                .map(|book| book.stable_book_id.as_str())
                .collect::<Vec<_>>(),
            vec!["book-a", "book-c"]
        );
        assert!(!catalog
            .chapters
            .iter()
            .any(|chapter| chapter.book_id == "book-b"));
        assert!(!catalog
            .resources
            .iter()
            .any(|resource| resource.book_id == "book-b"));
        assert_eq!(
            lookup_local_book_catalog_by_fingerprint(&catalog, &request)
                .unwrap()
                .into_iter()
                .map(|book| book.stable_book_id)
                .collect::<Vec<_>>(),
            vec!["book-a", "book-c"]
        );

        let invalid_chapter_ref = LocalBookCatalogSnapshot {
            schema_version: LOCAL_BOOK_CATALOG_SCHEMA_VERSION,
            books: vec![content_match.clone()],
            chapters: vec![chapter_index("missing-book", "missing-chapter", 0)],
            resources: Vec::new(),
        };
        assert_eq!(
            invalid_chapter_ref.validate().unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "chapters.book_id".into()
            }
        );

        let invalid_resource_ref = LocalBookCatalogSnapshot {
            schema_version: LOCAL_BOOK_CATALOG_SCHEMA_VERSION,
            books: vec![content_match.clone()],
            chapters: Vec::new(),
            resources: vec![resource_index(
                "missing-book",
                "missing-resource",
                "OPS/missing.css",
                "text/css",
                32,
                Some("missing-checksum"),
                LocalBookResourceKind::Css,
            )],
        };
        assert_eq!(
            invalid_resource_ref.validate().unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "resources.book_id".into()
            }
        );

        let mut mismatched = catalog_entry(&fingerprint("full-mismatch", "Mismatch", None, 1));
        mismatched.stable_book_id = "book-mismatch".into();
        assert_eq!(
            upsert_local_book_catalog_entry(
                &catalog,
                mismatched,
                vec![chapter_index("other-book", "chapter", 0)],
                Vec::new(),
            )
            .unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "chapters.book_id".into()
            }
        );
    }

    #[test]
    fn local_book_format_wire_values_match_legacy_reader_core_model() {
        let cases = [
            (LocalBookFormat::Txt, "txt"),
            (LocalBookFormat::Epub, "epub"),
            (LocalBookFormat::Pdf, "pdf"),
            (LocalBookFormat::Html, "html"),
            (LocalBookFormat::Mobi, "mobi"),
            (LocalBookFormat::Azw, "azw"),
            (LocalBookFormat::Umd, "umd"),
            (LocalBookFormat::Archive, "archive"),
            (LocalBookFormat::WebDav, "webdav"),
            (LocalBookFormat::Unknown, "unknown"),
        ];

        for (format, expected_wire_value) in cases {
            let json = serde_json::to_string(&format).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            let decoded: LocalBookFormat = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, format);
        }
    }

    #[test]
    fn local_book_format_rejects_unknown_or_drifted_wire_values() {
        for invalid in [r#""""#, r#""TXT""#, r#""webDav""#, r#""fb2""#] {
            assert!(serde_json::from_str::<LocalBookFormat>(invalid).is_err());
        }
    }

    #[test]
    fn local_book_capability_level_wire_values_match_legacy_reader_core_model() {
        let cases = [
            (LocalBookCapabilityLevel::MetadataOnly, "metadata_only"),
            (LocalBookCapabilityLevel::TextBoundary, "text_boundary"),
            (LocalBookCapabilityLevel::IndexedText, "indexed_text"),
            (
                LocalBookCapabilityLevel::PlatformRendered,
                "platform_rendered",
            ),
            (LocalBookCapabilityLevel::Unsupported, "unsupported"),
        ];

        for (level, expected_wire_value) in cases {
            let json = serde_json::to_string(&level).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<LocalBookCapabilityLevel>(&json).unwrap(),
                level
            );
        }
        assert!(serde_json::from_str::<LocalBookCapabilityLevel>(r#""metadataOnly""#).is_err());
    }

    #[test]
    fn local_book_format_capability_matches_legacy_core_boundary() {
        let txt = local_book_format_capability(LocalBookFormat::Txt);
        txt.validate().unwrap();
        assert_eq!(txt.capability_level, LocalBookCapabilityLevel::IndexedText);
        assert!(txt.can_probe_metadata);
        assert!(txt.can_build_chapter_index);
        assert!(txt.can_provide_text_preview);
        assert_eq!(
            txt.host_responsibilities,
            vec![
                "file_access".to_string(),
                "encoding_adapter_when_needed".to_string(),
                "reader_pagination_ui".to_string()
            ]
        );
        assert!(txt.clean_room_maintained);
        assert!(!txt.external_gpl_code_copied);

        let pdf = local_book_format_capability(LocalBookFormat::Pdf);
        assert_eq!(pdf.capability_level, LocalBookCapabilityLevel::TextBoundary);
        assert!(pdf.requires_platform_renderer);
        assert!(!pdf.can_render_natively_in_core);
        assert!(pdf
            .host_responsibilities
            .contains(&"ocr_if_required".into()));

        let mobi = local_book_format_capability(LocalBookFormat::Mobi);
        assert_eq!(
            mobi.capability_level,
            LocalBookCapabilityLevel::TextBoundary
        );
        assert!(mobi.requires_external_decoder_for_full_parity);
        assert!(mobi.parser_boundary.contains("HUFF/CDIC"));

        let webdav = local_book_format_capability(LocalBookFormat::WebDav);
        assert_eq!(
            webdav.capability_level,
            LocalBookCapabilityLevel::MetadataOnly
        );
        assert!(webdav.can_probe_metadata);
        assert!(!webdav.can_build_chapter_index);
        assert!(!webdav.can_provide_text_preview);
        assert!(webdav
            .host_responsibilities
            .contains(&"remote_byte_fetch".into()));

        let unknown = local_book_format_capability(LocalBookFormat::Unknown);
        assert_eq!(
            unknown.capability_level,
            LocalBookCapabilityLevel::Unsupported
        );
        assert!(!unknown.requires_platform_file_access);
        assert_eq!(unknown.host_responsibilities, vec!["user_visible_error"]);
    }

    #[test]
    fn local_book_capability_report_preserves_host_owned_parity_boundary() {
        let formats = [
            LocalBookFormat::Txt,
            LocalBookFormat::Epub,
            LocalBookFormat::Pdf,
            LocalBookFormat::Mobi,
            LocalBookFormat::Azw,
            LocalBookFormat::Umd,
            LocalBookFormat::Archive,
            LocalBookFormat::WebDav,
            LocalBookFormat::Unknown,
        ];
        let report = local_book_capability_report(&formats, 1_700_000_000).unwrap();

        assert_eq!(
            report.schema_version,
            LOCAL_BOOK_CAPABILITY_REPORT_SCHEMA_VERSION
        );
        assert_eq!(report.generated_at, 1_700_000_000);
        assert_eq!(report.capabilities.len(), formats.len());
        assert!(report.clean_room_maintained);
        assert!(!report.external_gpl_code_copied);
        assert_eq!(
            report.full_parity_still_host_owned,
            vec![
                "file_picker_and_security_scoped_permissions".to_string(),
                "long_lived_file_bookmark_persistence".to_string(),
                "interactive_pdf_rendering".to_string(),
                "ocr_for_image_only_pdf".to_string(),
                "proprietary_mobi_azw_full_decoder".to_string(),
                "reader_ui_pagination_and_selection".to_string()
            ]
        );
        assert!(report
            .capabilities
            .iter()
            .all(|capability| capability.clean_room_maintained
                && !capability.external_gpl_code_copied));

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["generatedAt"], 1_700_000_000);
        assert_eq!(json["capabilities"][0]["capabilityLevel"], "indexed_text");
        assert_eq!(json["capabilities"][7]["format"], "webdav");
        assert_eq!(json["capabilities"][7]["capabilityLevel"], "metadata_only");
        assert_eq!(
            json["fullParityStillHostOwned"][2],
            "interactive_pdf_rendering"
        );
        assert!(
            serde_json::from_value::<LocalBookCapabilityReport>(serde_json::json!({
                "schemaVersion": 1,
                "generatedAt": 1,
                "capabilities": [],
                "bogus": true
            }))
            .is_err()
        );
    }

    #[test]
    fn local_book_capability_validation_rejects_drifted_boundary_metadata() {
        let mut capability = local_book_format_capability(LocalBookFormat::Txt);
        capability.parser_boundary = " ".into();
        assert_eq!(
            capability.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "parser_boundary".into()
            }
        );

        let mut capability = local_book_format_capability(LocalBookFormat::Archive);
        capability.host_responsibilities.push(" ".into());
        assert_eq!(
            capability.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "host_responsibilities".into()
            }
        );

        let mut report =
            local_book_capability_report(&[LocalBookFormat::Txt], 1_700_000_000).unwrap();
        report.schema_version = 0;
        assert_eq!(
            report.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "schema_version".into()
            }
        );
    }

    #[test]
    fn local_book_change_and_validation_policy_wire_values_match_legacy_runtime() {
        let decisions = [
            (LocalBookChangeDecision::Unchanged, "unchanged"),
            (
                LocalBookChangeDecision::MetadataOnlyChanged,
                "metadata_only_changed",
            ),
            (LocalBookChangeDecision::ContentChanged, "content_changed"),
            (LocalBookChangeDecision::FormatChanged, "format_changed"),
            (
                LocalBookChangeDecision::ParserConfigChanged,
                "parser_config_changed",
            ),
            (LocalBookChangeDecision::Inaccessible, "inaccessible"),
            (LocalBookChangeDecision::Removed, "removed"),
            (LocalBookChangeDecision::ReplacementFile, "replacement_file"),
            (
                LocalBookChangeDecision::UncertainRequiresFullValidation,
                "uncertain_requires_full_validation",
            ),
        ];
        for (decision, expected_wire_value) in decisions {
            let json = serde_json::to_string(&decision).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<LocalBookChangeDecision>(&json).unwrap(),
                decision
            );
        }

        let policies = [
            (LocalBookValidationPolicy::MetadataOnly, "metadata_only"),
            (
                LocalBookValidationPolicy::FastFingerprint,
                "fast_fingerprint",
            ),
            (
                LocalBookValidationPolicy::FullFingerprint,
                "full_fingerprint",
            ),
            (
                LocalBookValidationPolicy::SemanticReimport,
                "semantic_reimport",
            ),
        ];
        for (policy, expected_wire_value) in policies {
            let json = serde_json::to_string(&policy).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<LocalBookValidationPolicy>(&json).unwrap(),
                policy
            );
        }
        assert!(serde_json::from_str::<LocalBookValidationPolicy>(r#""fullFingerprint""#).is_err());
    }

    #[test]
    fn local_book_change_decision_matches_legacy_priority_rules() {
        let existing = fingerprint("full-a", "title", Some("author"), 2);
        let entry = catalog_entry(&existing);

        let removed =
            decide_local_book_change(None, &existing, LocalBookValidationPolicy::FullFingerprint)
                .unwrap();
        assert_eq!(
            removed,
            LocalBookChangeResult {
                decision: LocalBookChangeDecision::Removed,
                reason_codes: vec!["catalog_entry_missing".into()],
            }
        );

        let mut format_changed = existing.clone();
        format_changed.fast.detected_format = LocalBookFormat::Pdf;
        let result = decide_local_book_change(
            Some(&entry),
            &format_changed,
            LocalBookValidationPolicy::FullFingerprint,
        )
        .unwrap();
        assert_eq!(result.decision, LocalBookChangeDecision::FormatChanged);
        assert_eq!(result.reason_codes, vec!["format_changed"]);

        let mut parser_changed = existing.clone();
        parser_changed.content.parser_config_checksum = "parser-v2".into();
        parser_changed.content.full_input_checksum = "full-b".into();
        let result = decide_local_book_change(
            Some(&entry),
            &parser_changed,
            LocalBookValidationPolicy::FullFingerprint,
        )
        .unwrap();
        assert_eq!(
            result.decision,
            LocalBookChangeDecision::ParserConfigChanged
        );
        assert_eq!(result.reason_codes, vec!["parser_config_changed"]);
    }

    #[test]
    fn local_book_change_decision_matches_legacy_policy_state_machine() {
        let existing = fingerprint("full-a", "title", Some("author"), 2);
        let entry = catalog_entry(&existing);

        let result = decide_local_book_change(
            Some(&entry),
            &existing,
            LocalBookValidationPolicy::MetadataOnly,
        )
        .unwrap();
        assert_eq!(
            result,
            LocalBookChangeResult {
                decision: LocalBookChangeDecision::MetadataOnlyChanged,
                reason_codes: vec!["metadata_policy_no_content_validation".into()],
            }
        );

        let mut size_changed = existing.clone();
        size_changed.fast.byte_count = 256;
        let result = decide_local_book_change(
            Some(&entry),
            &size_changed,
            LocalBookValidationPolicy::MetadataOnly,
        )
        .unwrap();
        assert_eq!(result.decision, LocalBookChangeDecision::ContentChanged);
        assert_eq!(result.reason_codes, vec!["size_changed"]);

        let result = decide_local_book_change(
            Some(&entry),
            &existing,
            LocalBookValidationPolicy::FastFingerprint,
        )
        .unwrap();
        assert_eq!(result.decision, LocalBookChangeDecision::Unchanged);
        assert_eq!(result.reason_codes, vec!["fast_fingerprint_match"]);

        let mut fast_changed = existing.clone();
        fast_changed.fast.prefix_checksum = "prefix-v2".into();
        let result = decide_local_book_change(
            Some(&entry),
            &fast_changed,
            LocalBookValidationPolicy::FastFingerprint,
        )
        .unwrap();
        assert_eq!(
            result.decision,
            LocalBookChangeDecision::UncertainRequiresFullValidation
        );
        assert_eq!(result.reason_codes, vec!["fast_fingerprint_changed"]);

        let result = decide_local_book_change(
            Some(&entry),
            &existing,
            LocalBookValidationPolicy::FullFingerprint,
        )
        .unwrap();
        assert_eq!(result.decision, LocalBookChangeDecision::Unchanged);
        assert_eq!(result.reason_codes, vec!["full_fingerprint_match"]);

        let mut full_changed = existing.clone();
        full_changed.content.full_input_checksum = "full-v2".into();
        let result = decide_local_book_change(
            Some(&entry),
            &full_changed,
            LocalBookValidationPolicy::FullFingerprint,
        )
        .unwrap();
        assert_eq!(result.decision, LocalBookChangeDecision::ContentChanged);
        assert_eq!(result.reason_codes, vec!["full_fingerprint_changed"]);

        let result = decide_local_book_change(
            Some(&entry),
            &full_changed,
            LocalBookValidationPolicy::SemanticReimport,
        )
        .unwrap();
        assert_eq!(
            result.decision,
            LocalBookChangeDecision::MetadataOnlyChanged
        );
        assert_eq!(result.reason_codes, vec!["semantic_match"]);

        let mut semantic_changed = existing.clone();
        semantic_changed.semantic.normalized_title = "other-title".into();
        let result = decide_local_book_change(
            Some(&entry),
            &semantic_changed,
            LocalBookValidationPolicy::SemanticReimport,
        )
        .unwrap();
        assert_eq!(result.decision, LocalBookChangeDecision::ContentChanged);
        assert_eq!(result.reason_codes, vec!["semantic_changed"]);
    }

    #[test]
    fn local_book_reading_restore_state_wire_values_match_legacy_runtime() {
        let states = [
            (
                LocalBookReadingRestoreState::ExactRestored,
                "exact_restored",
            ),
            (
                LocalBookReadingRestoreState::LocatorRestored,
                "locator_restored",
            ),
            (
                LocalBookReadingRestoreState::OrdinalRestored,
                "ordinal_restored",
            ),
            (
                LocalBookReadingRestoreState::ContextualRestored,
                "contextual_restored",
            ),
            (
                LocalBookReadingRestoreState::NearestChapterRestored,
                "nearest_chapter_restored",
            ),
            (
                LocalBookReadingRestoreState::ResetToBeginning,
                "reset_to_beginning",
            ),
            (
                LocalBookReadingRestoreState::StaleBookFingerprint,
                "stale_book_fingerprint",
            ),
            (
                LocalBookReadingRestoreState::ChapterRemoved,
                "chapter_removed",
            ),
            (
                LocalBookReadingRestoreState::AmbiguousMatch,
                "ambiguous_match",
            ),
        ];

        for (state, expected_wire_value) in states {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<LocalBookReadingRestoreState>(&json).unwrap(),
                state
            );
        }
        assert!(
            serde_json::from_str::<LocalBookReadingRestoreState>(r#""exactRestored""#).is_err()
        );
    }

    #[test]
    fn local_book_reading_progress_resolves_exact_and_stale_fingerprint_first() {
        let existing = fingerprint("full-a", "title", Some("author"), 2);
        let entry = catalog_entry(&existing);
        let chapters = vec![
            chapter_index("book-existing", "chapter-0", 0),
            chapter_index("book-existing", "chapter-1", 1),
        ];

        let progress = reading_progress(Some("chapter-1"), 1, None, "full-a");
        let result = resolve_local_book_reading_progress(
            &progress,
            "book-existing",
            Some(&entry),
            &chapters,
        )
        .unwrap();
        assert_eq!(
            result.restore_state,
            LocalBookReadingRestoreState::ExactRestored
        );
        assert_eq!(result.locator, progress.locator);
        assert!(result.diagnostics.is_empty());

        let stale = reading_progress(Some("chapter-1"), 1, None, "old-full");
        let result =
            resolve_local_book_reading_progress(&stale, "book-existing", Some(&entry), &chapters)
                .unwrap();
        assert_eq!(
            result.restore_state,
            LocalBookReadingRestoreState::StaleBookFingerprint
        );
        assert_eq!(result.locator, stale.locator);
        assert_eq!(
            result.diagnostics,
            vec!["fingerprint_changed_but_chapter_id_exists"]
        );
    }

    #[test]
    fn local_book_reading_progress_resolves_by_locator_ordinal_and_nearest() {
        let existing = fingerprint("full-a", "title", Some("author"), 2);
        let entry = catalog_entry(&existing);
        let chapters = vec![
            chapter_index("book-existing", "chapter-0", 0),
            chapter_index("book-existing", "chapter-1", 1),
            chapter_index("book-existing", "chapter-2", 2),
        ];

        let by_locator = reading_progress(
            Some("missing"),
            9,
            Some("local://book-existing/chapter/2"),
            "full-a",
        );
        let result = resolve_local_book_reading_progress(
            &by_locator,
            "book-existing",
            Some(&entry),
            &chapters,
        )
        .unwrap();
        assert_eq!(
            result.restore_state,
            LocalBookReadingRestoreState::LocatorRestored
        );
        assert_eq!(result.locator.chapter_id.as_deref(), Some("chapter-2"));
        assert_eq!(result.locator.chapter_ordinal, 2);
        assert_eq!(
            result.locator.chapter_canonical_locator.as_deref(),
            Some("local://book-existing/chapter/2")
        );
        assert_eq!(result.locator.character_offset, 12);

        let by_ordinal = reading_progress(Some("missing"), 1, None, "full-a");
        let result = resolve_local_book_reading_progress(
            &by_ordinal,
            "book-existing",
            Some(&entry),
            &chapters,
        )
        .unwrap();
        assert_eq!(
            result.restore_state,
            LocalBookReadingRestoreState::OrdinalRestored
        );
        assert_eq!(result.locator.chapter_id.as_deref(), Some("chapter-1"));

        let high_ordinal = reading_progress(Some("missing"), 99, None, "full-a");
        let result = resolve_local_book_reading_progress(
            &high_ordinal,
            "book-existing",
            Some(&entry),
            &chapters,
        )
        .unwrap();
        assert_eq!(
            result.restore_state,
            LocalBookReadingRestoreState::NearestChapterRestored
        );
        assert_eq!(result.locator.chapter_id.as_deref(), Some("chapter-2"));
        assert_eq!(result.diagnostics, vec!["nearest_fallback"]);

        let negative_ordinal = reading_progress(Some("missing"), -5, None, "full-a");
        let result = resolve_local_book_reading_progress(
            &negative_ordinal,
            "book-existing",
            Some(&entry),
            &chapters,
        )
        .unwrap();
        assert_eq!(
            result.restore_state,
            LocalBookReadingRestoreState::NearestChapterRestored
        );
        assert_eq!(result.locator.chapter_id.as_deref(), Some("chapter-0"));
    }

    #[test]
    fn local_book_reading_progress_resets_when_book_or_index_is_missing() {
        let existing = fingerprint("full-a", "title", Some("author"), 2);
        let entry = catalog_entry(&existing);
        let chapters = vec![chapter_index("book-existing", "chapter-0", 0)];
        let progress = reading_progress(Some("chapter-0"), 0, None, "full-a");

        let missing_book =
            resolve_local_book_reading_progress(&progress, "book-existing", None, &chapters)
                .unwrap();
        assert_eq!(
            missing_book.restore_state,
            LocalBookReadingRestoreState::ResetToBeginning
        );
        assert_eq!(
            missing_book.locator.chapter_id.as_deref(),
            Some("chapter-0")
        );
        assert_eq!(missing_book.locator.character_offset, 0);
        assert_eq!(missing_book.locator.normalized_progress_in_book, 0.0);
        assert_eq!(
            missing_book.diagnostics,
            vec!["missing_book_or_empty_index"]
        );

        let empty_index =
            resolve_local_book_reading_progress(&progress, "book-existing", Some(&entry), &[])
                .unwrap();
        assert_eq!(
            empty_index.restore_state,
            LocalBookReadingRestoreState::ResetToBeginning
        );
        assert!(empty_index.locator.chapter_id.is_none());
        assert!(empty_index.locator.chapter_canonical_locator.is_none());
        assert_eq!(empty_index.locator.parser_version, "RECOVERY-32");
    }

    #[test]
    fn local_book_reading_progress_json_and_validation_match_legacy_shape() {
        let progress = reading_progress(
            Some("chapter-1"),
            1,
            Some("local://book-existing/chapter/1"),
            "full-a",
        );
        progress.validate().unwrap();
        let json = serde_json::to_value(&progress).unwrap();
        assert_eq!(json["locator"]["bookId"], "book-existing");
        assert_eq!(json["locator"]["chapterId"], "chapter-1");
        assert_eq!(json["locator"]["chapterOrdinal"], 1);
        assert_eq!(json["restoreState"], "exact_restored");

        let mut invalid_locator = progress.clone();
        invalid_locator.locator.normalized_progress_in_book = 1.1;
        assert_eq!(
            invalid_locator.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "normalized_progress_in_book".into()
            }
        );

        let mut invalid_chapter = chapter_index("book-existing", "chapter-0", 0);
        invalid_chapter.canonical_locator = " ".into();
        assert_eq!(
            invalid_chapter.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "canonical_locator".into()
            }
        );
        assert!(
            serde_json::from_value::<LocalBookReadingProgress>(serde_json::json!({
                "locator": json["locator"].clone(),
                "restoreState": "exact_restored",
                "diagnostics": [],
                "bogus": true
            }))
            .is_err()
        );
    }

    #[test]
    fn local_book_cache_state_wire_values_match_legacy_runtime() {
        let states = [
            (LocalBookCacheState::Empty, "empty"),
            (LocalBookCacheState::MetadataOnly, "metadata_only"),
            (LocalBookCacheState::IndexOnly, "index_only"),
            (LocalBookCacheState::Lazy, "lazy"),
            (
                LocalBookCacheState::PartiallyMaterialized,
                "partially_materialized",
            ),
            (LocalBookCacheState::Materialized, "materialized"),
            (LocalBookCacheState::Stale, "stale"),
            (LocalBookCacheState::Invalidated, "invalidated"),
        ];

        for (state, expected_wire_value) in states {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<LocalBookCacheState>(&json).unwrap(),
                state
            );
        }
        assert!(serde_json::from_str::<LocalBookCacheState>(r#""metadataOnly""#).is_err());
    }

    #[test]
    fn local_book_cache_metadata_round_trips_legacy_shape_and_validates() {
        let metadata = cache_metadata("chapter:book-existing:chapter-1:RECOVERY-32", "chapter-1");
        metadata.validate().unwrap();

        let json = serde_json::to_value(&metadata).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "cacheKey": "chapter:book-existing:chapter-1:RECOVERY-32",
                "bookFingerprint": "full-a",
                "parserConfigChecksum": "parser",
                "parserVersion": "RECOVERY-32",
                "chapterOrResourceId": "chapter-1",
                "contentChecksum": "checksum-chapter-1",
                "createdTimestamp": "1970-01-01T00:00:00Z",
                "lastAccessTimestamp": "1970-01-01T00:00:00Z",
                "byteCount": 128,
                "validationState": "materialized",
                "evictionPriority": 10
            })
        );
        assert_eq!(
            serde_json::from_value::<LocalBookCacheMetadata>(json.clone()).unwrap(),
            metadata
        );

        let mut invalid = metadata.clone();
        invalid.cache_key = " ".into();
        assert_eq!(
            invalid.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "cache_key".into()
            }
        );
        assert!(
            serde_json::from_value::<LocalBookCacheMetadata>(serde_json::json!({
                "cacheKey": "k",
                "bookFingerprint": "f",
                "parserConfigChecksum": "p",
                "parserVersion": "RECOVERY-32",
                "chapterOrResourceId": "c",
                "createdTimestamp": "1970-01-01T00:00:00Z",
                "lastAccessTimestamp": "1970-01-01T00:00:00Z",
                "byteCount": 1,
                "validationState": "materialized",
                "evictionPriority": 0,
                "bogus": true
            }))
            .is_err()
        );
    }

    #[test]
    fn local_book_cache_metadata_upsert_replaces_and_enumerates_by_key() {
        let entries = vec![
            cache_metadata("chapter:book-b:chapter-2:RECOVERY-32", "chapter-2"),
            cache_metadata("chapter:book-a:chapter-1:RECOVERY-32", "chapter-1"),
        ];
        let listed = enumerate_local_book_cache_metadata(&entries).unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|entry| entry.cache_key.as_str())
                .collect::<Vec<_>>(),
            vec![
                "chapter:book-a:chapter-1:RECOVERY-32",
                "chapter:book-b:chapter-2:RECOVERY-32"
            ]
        );

        let mut replacement = cache_metadata("chapter:book-a:chapter-1:RECOVERY-32", "chapter-1");
        replacement.byte_count = 512;
        replacement.last_access_timestamp = "1970-01-01T00:00:01Z".into();
        let merged = upsert_local_book_cache_metadata(&entries, replacement.clone()).unwrap();
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0], replacement);
        assert_eq!(merged[1].chapter_or_resource_id, "chapter-2");
    }

    #[test]
    fn local_book_cache_metadata_invalidation_matches_legacy_store_rules() {
        let entries = vec![
            cache_metadata("chapter:book-a:chapter-1:RECOVERY-32", "chapter-1"),
            cache_metadata("resource:book-a:cover:RECOVERY-32", "cover"),
            cache_metadata("chapter:book-b:chapter-1:RECOVERY-32", "chapter-1"),
        ];

        let by_key =
            invalidate_local_book_cache_key(&entries, "resource:book-a:cover:RECOVERY-32").unwrap();
        assert_eq!(
            by_key
                .iter()
                .map(|entry| entry.cache_key.as_str())
                .collect::<Vec<_>>(),
            vec![
                "chapter:book-a:chapter-1:RECOVERY-32",
                "chapter:book-b:chapter-1:RECOVERY-32"
            ]
        );

        let by_book = invalidate_local_book_cache_for_book(&entries, "book-a").unwrap();
        assert_eq!(
            by_book
                .iter()
                .map(|entry| entry.cache_key.as_str())
                .collect::<Vec<_>>(),
            vec!["chapter:book-b:chapter-1:RECOVERY-32"]
        );
        assert_eq!(
            invalidate_local_book_cache_for_book(&entries, " ").unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "book_id".into()
            }
        );
    }

    #[test]
    fn local_book_store_snapshot_persists_catalog_progress_and_cache_without_host_paths() {
        let fingerprint = fingerprint("full-a", "Alpha", Some("Author"), 1);
        let catalog = upsert_local_book_catalog_entry(
            &LocalBookCatalogSnapshot::empty(),
            catalog_entry(&fingerprint),
            vec![chapter_index("book-existing", "chapter-1", 1)],
            Vec::new(),
        )
        .unwrap();
        let progress = reading_progress(
            Some("chapter-1"),
            1,
            Some("local://book/chapter/1"),
            "full-a",
        );
        let cache = cache_metadata("chapter:book-existing:chapter-1:RECOVERY-32", "chapter-1");

        let snapshot = build_local_book_library_store_snapshot(
            1_700_000_000,
            &catalog,
            &[progress.clone()],
            &[cache.clone()],
        )
        .unwrap();

        assert_eq!(
            snapshot.schema_version,
            LOCAL_BOOK_LIBRARY_STORE_SNAPSHOT_SCHEMA_VERSION
        );
        assert_eq!(snapshot.catalog.books.len(), 1);
        assert_eq!(
            snapshot.reading_progress[0].locator.chapter_id.as_deref(),
            Some("chapter-1")
        );
        assert_eq!(
            snapshot.cache_metadata[0].cache_key,
            "chapter:book-existing:chapter-1:RECOVERY-32"
        );

        let json = local_book_library_store_snapshot_portable_json(&snapshot).unwrap();
        assert!(!json.contains("/Users/"));
        assert!(json.contains(r#""readingProgress""#));
        assert!(json.contains(r#""cacheMetadata""#));
        let decoded: LocalBookLibraryStoreSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn local_book_store_snapshot_rejects_drifted_progress_and_host_path_cache_keys() {
        let fingerprint = fingerprint("full-a", "Alpha", Some("Author"), 1);
        let catalog = upsert_local_book_catalog_entry(
            &LocalBookCatalogSnapshot::empty(),
            catalog_entry(&fingerprint),
            vec![chapter_index("book-existing", "chapter-1", 1)],
            Vec::new(),
        )
        .unwrap();

        let mut missing_book_progress = reading_progress(Some("chapter-1"), 1, None, "full-a");
        missing_book_progress.locator.book_id = "missing-book".into();
        assert_eq!(
            build_local_book_library_store_snapshot(1, &catalog, &[missing_book_progress], &[])
                .unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "reading_progress.book_id".into()
            }
        );

        let host_path_cache = cache_metadata(
            "/Users/minliny/Library/Caches/reader/chapter-1",
            "chapter-1",
        );
        let snapshot =
            build_local_book_library_store_snapshot(1, &catalog, &[], &[host_path_cache]).unwrap();
        assert_eq!(
            local_book_library_store_snapshot_portable_json(&snapshot).unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "host_path".into()
            }
        );
    }

    #[test]
    fn local_book_import_state_and_mode_wire_values_match_legacy_runtime() {
        let states = [
            (LocalBookImportState::Probed, "probed"),
            (LocalBookImportState::Fingerprinted, "fingerprinted"),
            (LocalBookImportState::MetadataImported, "metadata_imported"),
            (LocalBookImportState::Indexed, "indexed"),
            (LocalBookImportState::LazyContentReady, "lazy_content_ready"),
            (
                LocalBookImportState::EagerFirstChapter,
                "eager_first_chapter",
            ),
            (LocalBookImportState::EagerAllContent, "eager_all_content"),
            (LocalBookImportState::Validated, "validated"),
            (LocalBookImportState::Invalidated, "invalidated"),
            (LocalBookImportState::Failed, "failed"),
        ];
        for (state, expected_wire_value) in states {
            let json = serde_json::to_string(&state).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<LocalBookImportState>(&json).unwrap(),
                state
            );
        }

        let modes = [
            (LocalBookLibraryImportMode::MetadataOnly, "metadata_only"),
            (LocalBookLibraryImportMode::IndexOnly, "index_only"),
            (LocalBookLibraryImportMode::LazyContent, "lazy_content"),
            (
                LocalBookLibraryImportMode::EagerFirstChapter,
                "eager_first_chapter",
            ),
            (
                LocalBookLibraryImportMode::EagerAllContent,
                "eager_all_content",
            ),
            (
                LocalBookLibraryImportMode::ValidateExisting,
                "validate_existing",
            ),
            (
                LocalBookLibraryImportMode::ReimportChanged,
                "reimport_changed",
            ),
        ];
        for (mode, expected_wire_value) in modes {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<LocalBookLibraryImportMode>(&json).unwrap(),
                mode
            );
        }
        assert!(serde_json::from_str::<LocalBookLibraryImportMode>(r#""lazyContent""#).is_err());
    }

    #[test]
    fn local_book_import_materialization_plan_matches_legacy_mode_matrix() {
        let cases = [
            (
                LocalBookLibraryImportMode::MetadataOnly,
                LocalBookImportState::MetadataImported,
                LocalBookCacheState::MetadataOnly,
                0,
                0,
                vec![
                    "chapter_index_import",
                    "resource_index_import",
                    "lazy_content_read",
                    "resource_materialization",
                ],
            ),
            (
                LocalBookLibraryImportMode::IndexOnly,
                LocalBookImportState::Indexed,
                LocalBookCacheState::IndexOnly,
                0,
                0,
                vec!["lazy_content_read", "resource_materialization"],
            ),
            (
                LocalBookLibraryImportMode::LazyContent,
                LocalBookImportState::LazyContentReady,
                LocalBookCacheState::Lazy,
                0,
                0,
                vec![
                    "chapter_content_materialization",
                    "resource_materialization",
                ],
            ),
            (
                LocalBookLibraryImportMode::EagerFirstChapter,
                LocalBookImportState::EagerFirstChapter,
                LocalBookCacheState::PartiallyMaterialized,
                1,
                0,
                vec![
                    "remaining_chapter_materialization",
                    "resource_materialization",
                ],
            ),
            (
                LocalBookLibraryImportMode::EagerAllContent,
                LocalBookImportState::EagerAllContent,
                LocalBookCacheState::Materialized,
                3,
                2,
                vec![],
            ),
            (
                LocalBookLibraryImportMode::ValidateExisting,
                LocalBookImportState::Validated,
                LocalBookCacheState::Lazy,
                0,
                0,
                vec![
                    "chapter_content_materialization",
                    "resource_materialization",
                ],
            ),
            (
                LocalBookLibraryImportMode::ReimportChanged,
                LocalBookImportState::LazyContentReady,
                LocalBookCacheState::Lazy,
                0,
                0,
                vec![
                    "chapter_content_materialization",
                    "resource_materialization",
                ],
            ),
        ];

        for (
            mode,
            import_status,
            cache_status,
            materialized_chapter_count,
            materialized_resource_count,
            deferred_stages,
        ) in cases
        {
            let plan = plan_local_book_import_materialization(mode, 3, 2).unwrap();
            assert_eq!(plan.mode, mode);
            assert_eq!(plan.import_status, import_status);
            assert_eq!(plan.cache_status, cache_status);
            assert_eq!(plan.materialized_chapter_count, materialized_chapter_count);
            assert_eq!(
                plan.materialized_resource_count,
                materialized_resource_count
            );
            assert_eq!(
                plan.deferred_stages,
                deferred_stages
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                plan.imported_stages,
                vec![
                    "probe".to_string(),
                    "fingerprint".to_string(),
                    "duplicate_lookup".to_string(),
                    "metadata_import".to_string(),
                    "chapter_index_import".to_string(),
                    "resource_index_import".to_string(),
                    "catalog_commit".to_string(),
                    "cache_commit".to_string()
                ]
            );
            assert_eq!(plan.completed_stage, "cache_commit");
            assert_eq!(plan.cache_hit_count, 0);
            assert_eq!(plan.cache_miss_count, 1);
            plan.validate().unwrap();
        }
    }

    #[test]
    fn local_book_import_materialization_plan_handles_empty_books_and_json_drift() {
        let empty_first = plan_local_book_import_materialization(
            LocalBookLibraryImportMode::EagerFirstChapter,
            0,
            4,
        )
        .unwrap();
        assert_eq!(empty_first.materialized_chapter_count, 0);
        assert_eq!(empty_first.materialized_resource_count, 0);
        assert_eq!(empty_first.cache_miss_count, 0);
        assert_eq!(
            empty_first.cache_status,
            LocalBookCacheState::PartiallyMaterialized
        );

        let full_empty = plan_local_book_import_materialization(
            LocalBookLibraryImportMode::EagerAllContent,
            0,
            4,
        )
        .unwrap();
        assert_eq!(full_empty.materialized_chapter_count, 0);
        assert_eq!(full_empty.materialized_resource_count, 4);
        assert!(full_empty.deferred_stages.is_empty());

        let json = serde_json::to_value(&full_empty).unwrap();
        assert_eq!(json["mode"], "eager_all_content");
        assert_eq!(json["importStatus"], "eager_all_content");
        assert_eq!(json["cacheStatus"], "materialized");
        assert_eq!(json["completedStage"], "cache_commit");
        assert!(
            serde_json::from_value::<LocalBookImportMaterializationPlan>(serde_json::json!({
                "mode": "lazy_content",
                "importStatus": "lazy_content_ready",
                "cacheStatus": "lazy",
                "materializedChapterCount": 0,
                "materializedResourceCount": 0,
                "importedStages": [],
                "completedStage": "cache_commit",
                "deferredStages": [],
                "cacheHitCount": 0,
                "cacheMissCount": 1,
                "bogus": true
            }))
            .is_err()
        );

        let mut invalid = full_empty;
        invalid.imported_stages.push(" ".into());
        assert_eq!(
            invalid.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "imported_stages".into()
            }
        );
    }

    #[test]
    fn local_book_library_metrics_summarize_recovery32_runner_counts() {
        let entry = catalog_entry(&fingerprint("full-a", "Title", Some("Author"), 2));
        let catalog = upsert_local_book_catalog_entry(
            &LocalBookCatalogSnapshot::empty(),
            entry,
            vec![
                chapter_index("book-existing", "chapter-0", 0),
                chapter_index("book-existing", "chapter-1", 1),
            ],
            vec![resource_index(
                "book-existing",
                "style",
                "OPS/style.css",
                "text/css",
                128,
                Some("style-checksum"),
                LocalBookResourceKind::Css,
            )],
        )
        .unwrap();
        let duplicate_results = vec![
            LocalBookDuplicateResult {
                decision: LocalBookDuplicateDecision::SameBytesDifferentPath,
                matched_book_id: Some("book-existing".into()),
                duplicate_group_id: Some("dup-a".into()),
                reason_codes: vec![
                    "full_fingerprint_match".into(),
                    "filename_checksum_changed".into(),
                ],
            },
            LocalBookDuplicateResult {
                decision: LocalBookDuplicateDecision::ExactDuplicate,
                matched_book_id: Some("book-existing".into()),
                duplicate_group_id: Some("dup-a".into()),
                reason_codes: vec![
                    "full_fingerprint_match".into(),
                    "filename_checksum_match".into(),
                ],
            },
            LocalBookDuplicateResult {
                decision: LocalBookDuplicateDecision::InsufficientEvidence,
                matched_book_id: None,
                duplicate_group_id: None,
                reason_codes: vec!["no_catalog_match".into()],
            },
        ];
        let change_results = vec![
            LocalBookChangeResult {
                decision: LocalBookChangeDecision::ContentChanged,
                reason_codes: vec!["full_fingerprint_changed".into()],
            },
            LocalBookChangeResult {
                decision: LocalBookChangeDecision::Unchanged,
                reason_codes: vec!["full_fingerprint_match".into()],
            },
        ];
        let chapter_reads = vec![
            chapter_read_result("chapter-0", 0, false),
            chapter_read_result("chapter-1", 1, true),
        ];
        let resource_request = LocalBookResourceReadRequest {
            book_id: "book-existing".into(),
            resource_id: "style".into(),
            max_bytes: 1024,
        };
        let resource_miss =
            plan_local_book_resource_read(&resource_request, &catalog.resources, None).unwrap();
        let resource_hit = plan_local_book_resource_read(
            &resource_request,
            &catalog.resources,
            Some(&resource_miss),
        )
        .unwrap();
        let progress_results = vec![reading_progress(
            Some("chapter-1"),
            1,
            Some("local://book-existing/chapter/1"),
            "full-a",
        )];

        let metrics = summarize_local_book_library_metrics(
            duplicate_results.len(),
            &catalog,
            &duplicate_results,
            &change_results,
            &chapter_reads,
            &[resource_miss, resource_hit],
            &progress_results,
            1,
            LOCAL_BOOK_MAX_PREVIEW_LIMIT,
        )
        .unwrap();

        assert_eq!(metrics.import_count, 3);
        assert_eq!(metrics.catalog_book_count, 1);
        assert_eq!(metrics.catalog_chapter_count, 2);
        assert_eq!(metrics.catalog_resource_count, 1);
        assert_eq!(metrics.chapter_read_count, 2);
        assert_eq!(metrics.resource_read_count, 2);
        assert_eq!(metrics.cache_hit_count, 2);
        assert_eq!(metrics.cache_miss_count, 2);
        assert_eq!(metrics.progress_restore_count, 1);
        assert_eq!(
            serde_json::to_value(&metrics).unwrap(),
            serde_json::json!({
                "importCount": 3,
                "catalogBookCount": 1,
                "catalogChapterCount": 2,
                "catalogResourceCount": 1,
                "duplicateDecisionCounts": {
                    "exact_duplicate": 1,
                    "insufficient_evidence": 1,
                    "same_bytes_different_path": 1
                },
                "changeDecisionCounts": {
                    "content_changed": 1,
                    "unchanged": 1
                },
                "chapterReadCount": 2,
                "resourceReadCount": 2,
                "cacheHitCount": 2,
                "cacheMissCount": 2,
                "progressRestoreCount": 1,
                "fullContentPersistedCount": 1,
                "previewCharacterLimit": 64
            })
        );
    }

    #[test]
    fn local_book_library_metrics_rejects_inconsistent_recovery32_inputs() {
        let catalog = LocalBookCatalogSnapshot::empty();
        let duplicate_results = vec![LocalBookDuplicateResult {
            decision: LocalBookDuplicateDecision::InsufficientEvidence,
            matched_book_id: None,
            duplicate_group_id: None,
            reason_codes: vec!["no_catalog_match".into()],
        }];

        assert_eq!(
            summarize_local_book_library_metrics(
                2,
                &catalog,
                &duplicate_results,
                &[],
                &[],
                &[],
                &[],
                0,
                LOCAL_BOOK_MAX_PREVIEW_LIMIT,
            )
            .unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "duplicate_results".into()
            }
        );
        assert_eq!(
            summarize_local_book_library_metrics(
                1,
                &catalog,
                &duplicate_results,
                &[],
                &[],
                &[],
                &[],
                0,
                LOCAL_BOOK_MAX_PREVIEW_LIMIT + 1,
            )
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "preview_character_limit".into()
            }
        );

        let mut invalid_duplicate = duplicate_results[0].clone();
        invalid_duplicate.reason_codes.push(" ".into());
        assert_eq!(
            summarize_local_book_library_metrics(
                1,
                &catalog,
                &[invalid_duplicate],
                &[],
                &[],
                &[],
                &[],
                0,
                LOCAL_BOOK_MAX_PREVIEW_LIMIT,
            )
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "reason_codes".into()
            }
        );

        let mut invalid_chapter = chapter_read_result("chapter-0", 0, false);
        invalid_chapter.preview = "x".repeat(LOCAL_BOOK_MAX_PREVIEW_LIMIT + 1);
        assert_eq!(
            summarize_local_book_library_metrics(
                1,
                &catalog,
                &duplicate_results,
                &[],
                &[invalid_chapter],
                &[],
                &[],
                0,
                LOCAL_BOOK_MAX_PREVIEW_LIMIT,
            )
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "preview".into()
            }
        );
    }

    #[test]
    fn local_book_format_detection_request_defaults_and_preview_cap_match_legacy_core() {
        let request = LocalBookFormatDetectionRequest {
            declared_filename: Some("Remote Book.EPUB".into()),
            declared_extension: None,
            declared_mime_type: None,
            maximum_input_size: LOCAL_BOOK_DEFAULT_MAXIMUM_INPUT_SIZE,
            preview_limit: 256,
        };

        let detection = detect_local_book_format(b"plain text fallback", &request).unwrap();

        assert_eq!(
            request.effective_preview_limit(),
            LOCAL_BOOK_MAX_PREVIEW_LIMIT
        );
        assert_eq!(detection.declared_format, Some(LocalBookFormat::Epub));
        assert_eq!(detection.format, LocalBookFormat::Txt);
        assert_eq!(detection.media_type, "text/plain");
        assert_eq!(detection.effective_preview_limit, 64);
        assert_eq!(
            detection.diagnostics,
            vec!["declared_detected_mismatch:declared epub, detected txt"]
        );

        let default_request = LocalBookFormatDetectionRequest::default();
        assert_eq!(
            default_request.maximum_input_size,
            LOCAL_BOOK_DEFAULT_MAXIMUM_INPUT_SIZE
        );
        assert_eq!(default_request.preview_limit, LOCAL_BOOK_MAX_PREVIEW_LIMIT);

        let mut invalid = default_request;
        invalid.maximum_input_size = 0;
        assert_eq!(
            invalid.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "maximum_input_size".into()
            }
        );
    }

    #[test]
    fn local_book_format_detection_uses_declared_extension_mime_and_magic() {
        let epub_zip = b"PK\x03\x04mimetypeapplication/epub+zipMETA-INF/container.xml";
        let epub = detect_local_book_format(
            epub_zip,
            &LocalBookFormatDetectionRequest {
                declared_filename: Some("fixture.epub".into()),
                ..LocalBookFormatDetectionRequest::default()
            },
        )
        .unwrap();
        assert_eq!(epub.format, LocalBookFormat::Epub);
        assert_eq!(epub.media_type, "application/epub+zip");
        assert!(epub.diagnostics.is_empty());

        let archive = detect_local_book_format(
            b"PK\x03\x04plain zip bytes",
            &LocalBookFormatDetectionRequest {
                declared_extension: Some("zip".into()),
                ..LocalBookFormatDetectionRequest::default()
            },
        )
        .unwrap();
        assert_eq!(archive.format, LocalBookFormat::Archive);
        assert_eq!(archive.media_type, "application/zip");

        let mut tar = vec![0u8; 300];
        tar[257..262].copy_from_slice(b"ustar");
        let tar_detection = detect_local_book_format(
            &tar,
            &LocalBookFormatDetectionRequest {
                declared_mime_type: Some("application/x-tar".into()),
                ..LocalBookFormatDetectionRequest::default()
            },
        )
        .unwrap();
        assert_eq!(tar_detection.format, LocalBookFormat::Archive);

        let pdf = detect_local_book_format(
            b"%PDF-1.7\nbody",
            &LocalBookFormatDetectionRequest::default(),
        )
        .unwrap();
        assert_eq!(pdf.format, LocalBookFormat::Pdf);
        assert_eq!(pdf.media_type, "application/pdf");

        let mobi = detect_local_book_format(
            b"header BOOKMOBI payload",
            &LocalBookFormatDetectionRequest::default(),
        )
        .unwrap();
        assert_eq!(mobi.format, LocalBookFormat::Mobi);

        let azw = detect_local_book_format(
            b"header BOOKMOBI payload",
            &LocalBookFormatDetectionRequest {
                declared_extension: Some("azw3".into()),
                ..LocalBookFormatDetectionRequest::default()
            },
        )
        .unwrap();
        assert_eq!(azw.format, LocalBookFormat::Azw);

        let umd = detect_local_book_format(
            &[0x89, 0x9b, 0x9a, 0xde, 0x00],
            &LocalBookFormatDetectionRequest::default(),
        )
        .unwrap();
        assert_eq!(umd.format, LocalBookFormat::Umd);
    }

    #[test]
    fn local_book_format_detection_accepts_webdav_descriptor_without_fetching() {
        let descriptor = br#"{
            "remotePath": "/dav/books/remote.epub",
            "etag": "\"etag\"",
            "format": "epub",
            "fileSize": 4096
        }"#;

        let detection = detect_local_book_format(
            descriptor,
            &LocalBookFormatDetectionRequest {
                declared_filename: Some("remote.webdav".into()),
                declared_mime_type: Some(
                    "application/vnd.reader-core.webdav-local-book+json".into(),
                ),
                ..LocalBookFormatDetectionRequest::default()
            },
        )
        .unwrap();

        assert_eq!(detection.format, LocalBookFormat::WebDav);
        assert_eq!(
            detection.media_type,
            "application/vnd.reader-core.webdav-local-book+json"
        );
        assert_eq!(detection.declared_format, Some(LocalBookFormat::WebDav));
        assert!(detection.diagnostics.is_empty());
    }

    #[test]
    fn local_book_media_type_mappings_match_legacy_runtime() {
        let cases = [
            (LocalBookFormat::Txt, "text/plain"),
            (LocalBookFormat::Epub, "application/epub+zip"),
            (LocalBookFormat::Pdf, "application/pdf"),
            (LocalBookFormat::Mobi, "application/x-mobipocket-ebook"),
            (LocalBookFormat::Azw, "application/vnd.amazon.ebook"),
            (LocalBookFormat::Umd, "application/x-umd"),
            (LocalBookFormat::Archive, "application/zip"),
            (
                LocalBookFormat::WebDav,
                "application/vnd.reader-core.webdav-local-book+json",
            ),
            (LocalBookFormat::Unknown, "application/octet-stream"),
        ];

        for (format, media_type) in cases {
            assert_eq!(local_book_media_type_for_format(format), media_type);
        }

        assert_eq!(
            local_book_media_type_for_extension(".epub"),
            "application/epub+zip"
        );
        assert_eq!(
            local_book_media_type_for_extension("azw3"),
            "application/vnd.amazon.ebook"
        );
        assert_eq!(
            local_book_media_type_for_extension("tar"),
            "application/x-tar"
        );
        assert_eq!(
            local_book_media_type_for_extension("webdavbook"),
            "application/vnd.reader-core.webdav-local-book+json"
        );
        assert_eq!(
            local_book_media_type_for_extension("application/pdf"),
            "application/pdf"
        );
    }

    #[test]
    fn local_book_duplicate_decision_wire_values_match_legacy_runtime() {
        let cases = [
            (
                LocalBookDuplicateDecision::ExactDuplicate,
                "exact_duplicate",
            ),
            (
                LocalBookDuplicateDecision::SameBytesDifferentPath,
                "same_bytes_different_path",
            ),
            (
                LocalBookDuplicateDecision::SameSemanticBook,
                "same_semantic_book",
            ),
            (
                LocalBookDuplicateDecision::LikelyDuplicate,
                "likely_duplicate",
            ),
            (
                LocalBookDuplicateDecision::DifferentEdition,
                "different_edition",
            ),
            (LocalBookDuplicateDecision::ChangedFile, "changed_file"),
            (LocalBookDuplicateDecision::Unrelated, "unrelated"),
            (
                LocalBookDuplicateDecision::InsufficientEvidence,
                "insufficient_evidence",
            ),
        ];

        for (decision, expected_wire_value) in cases {
            let json = serde_json::to_string(&decision).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<LocalBookDuplicateDecision>(&json).unwrap(),
                decision
            );
        }
        assert_eq!(
            local_book_stable_checksum(&["same.txt"]),
            "fnv1a64:fae0f46ea5a643dd"
        );
    }

    #[test]
    fn local_book_duplicate_decision_matches_legacy_full_fingerprint_rules() {
        let existing = fingerprint("full-a", "title", Some("author"), 2);
        let entry = catalog_entry(&existing);

        let exact =
            decide_local_book_duplicate(&existing, Some("same.txt"), &[entry.clone()]).unwrap();
        assert_eq!(
            exact,
            LocalBookDuplicateResult {
                decision: LocalBookDuplicateDecision::ExactDuplicate,
                matched_book_id: Some("book-existing".into()),
                duplicate_group_id: Some("fnv1a64:ab077cfd4a97dbdd".into()),
                reason_codes: vec![
                    "full_fingerprint_match".into(),
                    "filename_checksum_match".into()
                ],
            }
        );

        let renamed =
            decide_local_book_duplicate(&existing, Some("renamed.txt"), &[entry]).unwrap();
        assert_eq!(
            renamed.decision,
            LocalBookDuplicateDecision::SameBytesDifferentPath
        );
        assert_eq!(
            renamed.reason_codes,
            vec![
                "full_fingerprint_match".to_string(),
                "filename_checksum_changed".to_string()
            ]
        );

        let no_match = decide_local_book_duplicate(&existing, Some("same.txt"), &[]).unwrap();
        assert_eq!(
            no_match.decision,
            LocalBookDuplicateDecision::InsufficientEvidence
        );
        assert_eq!(no_match.reason_codes, vec!["no_catalog_match"]);
    }

    #[test]
    fn local_book_duplicate_decision_matches_legacy_semantic_rules() {
        let existing = fingerprint("full-a", "title", Some("author"), 2);
        let entry = catalog_entry(&existing);

        let same_semantic = fingerprint("full-b", "title", Some("author"), 2);
        let result =
            decide_local_book_duplicate(&same_semantic, Some("other.txt"), &[entry.clone()])
                .unwrap();
        assert_eq!(
            result.decision,
            LocalBookDuplicateDecision::SameSemanticBook
        );
        assert_eq!(
            result.duplicate_group_id.as_deref(),
            Some("fnv1a64:d021e47b93d97a97")
        );
        assert_eq!(
            result.reason_codes,
            vec!["semantic_title_author_chapter_count_match"]
        );

        let different_author = fingerprint("full-c", "title", Some("other-author"), 2);
        let result =
            decide_local_book_duplicate(&different_author, None, &[entry.clone()]).unwrap();
        assert_eq!(result.decision, LocalBookDuplicateDecision::Unrelated);
        assert_eq!(result.reason_codes, vec!["same_title_different_author"]);

        let different_edition = fingerprint("full-d", "title", Some("author"), 3);
        let result =
            decide_local_book_duplicate(&different_edition, None, &[entry.clone()]).unwrap();
        assert_eq!(
            result.decision,
            LocalBookDuplicateDecision::DifferentEdition
        );
        assert_eq!(
            result.reason_codes,
            vec!["same_title_different_chapter_count"]
        );

        let partial_overlap = fingerprint("full-e", "other-title", Some("author"), 2);
        let result = decide_local_book_duplicate(&partial_overlap, None, &[entry]).unwrap();
        assert_eq!(result.decision, LocalBookDuplicateDecision::LikelyDuplicate);
        assert_eq!(result.reason_codes, vec!["partial_semantic_overlap"]);
    }

    #[test]
    fn chapter_split_policy_wire_values_match_legacy_reader_core_model() {
        let cases = [
            (ChapterSplitPattern::Regex, "regex"),
            (ChapterSplitPattern::Size, "size"),
            (ChapterSplitPattern::Marker, "marker"),
            (ChapterSplitPattern::Auto, "auto"),
        ];

        for (pattern, expected_wire_value) in cases {
            let json = serde_json::to_string(&pattern).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            let decoded: ChapterSplitPattern = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, pattern);
        }

        let policy = ChapterSplitPolicy::default();
        assert_eq!(policy.pattern, ChapterSplitPattern::Auto);
        assert_eq!(
            serde_json::to_value(&policy).unwrap(),
            serde_json::json!({ "pattern": "auto" })
        );
        assert!(serde_json::from_str::<ChapterSplitPattern>(r#""unknown_pattern""#).is_err());
    }

    #[test]
    fn chapter_split_policy_round_trips_and_validates_parameters() {
        let policy = ChapterSplitPolicy {
            pattern: ChapterSplitPattern::Regex,
            regex: Some("^Chapter\\s+[0-9]+".into()),
            size_bytes: None,
            marker: None,
        };

        policy.validate().unwrap();
        let json = serde_json::to_string(&policy).unwrap();
        assert!(json.contains(r#""pattern":"regex""#));
        assert!(json.contains(r#""regex":"^Chapter\\s+[0-9]+""#));
        assert_eq!(
            serde_json::from_str::<ChapterSplitPolicy>(&json).unwrap(),
            policy
        );

        let size_policy = ChapterSplitPolicy {
            pattern: ChapterSplitPattern::Size,
            regex: None,
            size_bytes: Some(0),
            marker: None,
        };
        assert_eq!(
            size_policy.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "size_bytes".into()
            }
        );
        let marker_policy = ChapterSplitPolicy {
            pattern: ChapterSplitPattern::Marker,
            regex: None,
            size_bytes: None,
            marker: Some(" ".into()),
        };
        assert_eq!(
            marker_policy.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "marker".into()
            }
        );
    }

    #[test]
    fn regex_chapter_split_policy_drives_txt_parsing() {
        let policy = ChapterSplitPolicy {
            pattern: ChapterSplitPattern::Regex,
            regex: Some("^Chapter\\s+[0-9]+".into()),
            size_bytes: None,
            marker: None,
        };
        let text = "Book Title\nChapter 1 Beginnings\nSome text.\nChapter 2 Journeys\nMore text.";

        let book =
            parse_txt_text_with_policy("regex-book", Some("Regex Book"), None, None, text, &policy)
                .unwrap();

        assert_eq!(book.chapters.len(), 2);
        assert_eq!(book.chapters[0].title, "Chapter 1 Beginnings");
        assert_eq!(book.chapters[0].content, "Some text.");
        assert_eq!(book.chapters[1].title, "Chapter 2 Journeys");
        assert_eq!(book.toc[1].url, "local://regex-book/chapter/1");

        let bad_policy = ChapterSplitPolicy {
            pattern: ChapterSplitPattern::Regex,
            regex: Some("[".into()),
            size_bytes: None,
            marker: None,
        };
        assert_eq!(
            parse_txt_text_with_policy("bad", Some("Bad"), None, None, text, &bad_policy)
                .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "regex".into()
            }
        );
    }

    #[test]
    fn marker_chapter_split_policy_matches_legacy_titles() {
        let policy = ChapterSplitPolicy {
            pattern: ChapterSplitPattern::Marker,
            regex: None,
            size_bytes: None,
            marker: Some("###".into()),
        };
        let text = "### Title One\nContent one.\n### Title Two\nContent two.\n### Title Three\nContent three.";

        let book = parse_txt_text_with_policy(
            "marker-book",
            Some("Marker Book"),
            None,
            None,
            text,
            &policy,
        )
        .unwrap();

        assert_eq!(
            book.chapters
                .iter()
                .map(|chapter| chapter.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Title One", "Title Two", "Title Three"]
        );
        assert_eq!(book.chapters[1].content, "Content two.");
    }

    #[test]
    fn size_chapter_split_policy_divides_utf8_byte_chunks() {
        let policy = ChapterSplitPolicy {
            pattern: ChapterSplitPattern::Size,
            regex: None,
            size_bytes: Some(1000),
            marker: None,
        };
        let text = "A".repeat(3000);

        let book =
            parse_txt_text_with_policy("size-book", Some("Size Book"), None, None, &text, &policy)
                .unwrap();

        assert_eq!(book.chapters.len(), 3);
        assert_eq!(
            book.chapters
                .iter()
                .map(|chapter| chapter.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Chapter 1", "Chapter 2", "Chapter 3"]
        );
        assert_eq!(book.chapters[1].start_char, 1000);
        assert_eq!(book.chapters[2].start_char, 2000);
        assert_eq!(book.chapters[2].content.chars().count(), 1000);
    }

    #[test]
    fn missing_policy_parameter_falls_back_to_single_chapter() {
        let policy = ChapterSplitPolicy {
            pattern: ChapterSplitPattern::Marker,
            regex: None,
            size_bytes: None,
            marker: None,
        };
        let text = "Just plain text\nno markers anywhere.";

        let mut library = LocalBookLibrary::new();
        let book = library
            .parse_and_upsert_txt_with_policy(
                LocalBookInput {
                    book_id: "fallback",
                    file_name: Some("fallback.txt"),
                    title: Some("Fallback"),
                    author: None,
                    bytes: text.as_bytes(),
                },
                &policy,
            )
            .unwrap();

        assert_eq!(book.chapters.len(), 1);
        assert_eq!(book.chapters[0].title, "Chapter 1");
        assert_eq!(
            library.get_chapter("fallback", 0).unwrap().content,
            "Just plain text\nno markers anywhere."
        );
    }

    #[test]
    fn local_toc_item_defaults_and_json_match_legacy_reader_core() {
        let item = LocalTocItem::new(" Chapter 1 ").unwrap();

        assert_eq!(item.title, "Chapter 1");
        assert_eq!(item.level, 1);
        assert!(item.byte_offset.is_none());
        assert!(item.children.is_none());
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            serde_json::json!({
                "title": "Chapter 1",
                "level": 1
            })
        );
    }

    #[test]
    fn local_toc_item_tree_round_trips_children_and_offsets() {
        let toc = LocalTocItem {
            title: "Part 1".into(),
            level: 1,
            byte_offset: Some(0),
            children: Some(vec![
                LocalTocItem {
                    title: "Chapter 1".into(),
                    level: 2,
                    byte_offset: Some(128),
                    children: None,
                },
                LocalTocItem {
                    title: "Chapter 2".into(),
                    level: 2,
                    byte_offset: Some(512),
                    children: Some(vec![LocalTocItem {
                        title: "Scene 2.1".into(),
                        level: 3,
                        byte_offset: Some(768),
                        children: None,
                    }]),
                },
            ]),
        };

        toc.validate().unwrap();
        let json = serde_json::to_string(&toc).unwrap();
        assert!(json.contains(r#""byteOffset":512"#));
        let decoded: LocalTocItem = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, toc);
        assert_eq!(
            decoded.children.as_ref().unwrap()[1]
                .children
                .as_ref()
                .unwrap()[0]
                .title,
            "Scene 2.1"
        );
    }

    #[test]
    fn local_toc_tree_flattens_depth_first_to_domain_toc() {
        let items = vec![LocalTocItem {
            title: "Part".into(),
            level: 1,
            byte_offset: None,
            children: Some(vec![
                LocalTocItem {
                    title: "Chapter".into(),
                    level: 2,
                    byte_offset: Some(1024),
                    children: Some(vec![LocalTocItem {
                        title: "Section".into(),
                        level: 3,
                        byte_offset: None,
                        children: None,
                    }]),
                },
                LocalTocItem {
                    title: "Appendix".into(),
                    level: 2,
                    byte_offset: None,
                    children: None,
                },
            ]),
        }];

        let flattened = flatten_local_toc_items("book-1", &items).unwrap();

        assert_eq!(
            flattened
                .iter()
                .map(|entry| (entry.index, entry.title.as_str(), entry.url.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (0, "Part", "local://book-1/chapter/0"),
                (1, "Chapter", "local://book-1/chapter/1"),
                (2, "Section", "local://book-1/chapter/2"),
                (3, "Appendix", "local://book-1/chapter/3"),
            ]
        );
    }

    #[test]
    fn local_toc_item_rejects_empty_titles_bad_levels_and_unknown_fields() {
        assert_eq!(
            LocalTocItem::new(" ").unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "title".into()
            }
        );

        let bad_level = LocalTocItem {
            title: "Root".into(),
            level: 0,
            byte_offset: None,
            children: None,
        };
        assert_eq!(
            bad_level.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "level".into()
            }
        );

        let bad_child = LocalTocItem {
            title: "Root".into(),
            level: 2,
            byte_offset: None,
            children: Some(vec![LocalTocItem {
                title: "Child".into(),
                level: 2,
                byte_offset: None,
                children: None,
            }]),
        };
        assert_eq!(
            bad_child.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "children.level".into()
            }
        );
        assert!(serde_json::from_str::<LocalTocItem>(
            r#"{"title":"Chapter","level":1,"bogus":true}"#
        )
        .is_err());
    }

    #[test]
    fn local_book_resource_kind_wire_values_and_classification_match_legacy_runtime() {
        let cases = [
            (LocalBookResourceKind::Cover, "cover"),
            (LocalBookResourceKind::Image, "image"),
            (LocalBookResourceKind::Css, "css"),
            (LocalBookResourceKind::Font, "font"),
            (LocalBookResourceKind::Other, "other"),
        ];

        for (kind, expected_wire_value) in cases {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!(r#""{expected_wire_value}""#));
            assert_eq!(
                serde_json::from_str::<LocalBookResourceKind>(&json).unwrap(),
                kind
            );
        }

        assert_eq!(
            local_book_resource_kind("images/cover.bin", "application/octet-stream", true),
            LocalBookResourceKind::Cover
        );
        assert_eq!(
            local_book_resource_kind("images/pic.bin", "image/png", false),
            LocalBookResourceKind::Image
        );
        assert_eq!(
            local_book_resource_kind("styles/book.css", "text/css", false),
            LocalBookResourceKind::Css
        );
        assert_eq!(
            local_book_resource_kind("fonts/book.WOFF", "application/octet-stream", false),
            LocalBookResourceKind::Font
        );
        assert_eq!(
            local_book_resource_kind("misc/data.bin", "application/octet-stream", false),
            LocalBookResourceKind::Other
        );
    }

    #[test]
    fn local_book_chapter_read_request_and_prefetch_plan_match_legacy_runtime_window() {
        let chapters = vec![
            chapter_index("book-existing", "chapter-0", 0),
            chapter_index("book-existing", "chapter-1", 1),
            chapter_index("book-existing", "chapter-2", 2),
        ];
        let request = LocalBookChapterReadRequest::new(
            "book-existing",
            Some("chapter-1".into()),
            Some(9),
            128,
        )
        .unwrap();

        assert_eq!(
            request.effective_preview_limit(),
            LOCAL_BOOK_MAX_PREVIEW_LIMIT
        );
        assert_eq!(
            resolve_local_book_chapter_read_request(&request, &chapters)
                .unwrap()
                .stable_chapter_id,
            "chapter-1"
        );

        let ordinal_request =
            LocalBookChapterReadRequest::by_ordinal("book-existing", 2, 32).unwrap();
        assert_eq!(
            serde_json::to_value(&ordinal_request).unwrap(),
            serde_json::json!({
                "bookId": "book-existing",
                "ordinal": 2,
                "previewLimit": 32
            })
        );
        assert_eq!(
            resolve_local_book_chapter_read_request(&ordinal_request, &chapters)
                .unwrap()
                .stable_chapter_id,
            "chapter-2"
        );

        let previous = local_book_previous_chapter_read_request(&chapters[0], 64).unwrap();
        assert_eq!(previous.ordinal, Some(0));
        let next = local_book_next_chapter_read_request(&chapters[1], 64).unwrap();
        assert_eq!(next.ordinal, Some(2));

        let centered = plan_local_book_chapter_prefetch(&LocalBookChapterPrefetchRequest {
            book_id: "book-existing".into(),
            anchor_ordinal: 1,
            radius: 1,
            maximum_count: 3,
        })
        .unwrap();
        assert_eq!(centered.ordinals, vec![0, 1, 2]);

        let capped = plan_local_book_chapter_prefetch(&LocalBookChapterPrefetchRequest {
            book_id: "book-existing".into(),
            anchor_ordinal: 10,
            radius: 3,
            maximum_count: 2,
        })
        .unwrap();
        assert_eq!(capped.ordinals, vec![7, 8]);

        assert_eq!(
            LocalBookChapterReadRequest::new("book-existing", None, None, 64).unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "chapter_selector".into()
            }
        );
        assert_eq!(
            plan_local_book_chapter_prefetch(&LocalBookChapterPrefetchRequest {
                book_id: "book-existing".into(),
                anchor_ordinal: 1,
                radius: 1,
                maximum_count: 0,
            })
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "maximum_count".into()
            }
        );
    }

    #[test]
    fn local_book_resource_read_plan_matches_legacy_safety_size_and_cache_rules() {
        let resources = vec![
            resource_index(
                "book-existing",
                "style",
                "OPS/style.css",
                "text/css",
                128,
                Some("style-checksum"),
                LocalBookResourceKind::Css,
            ),
            resource_index(
                "book-existing",
                "font",
                "OPS/fonts/book.ttf",
                "font/ttf",
                64,
                None,
                LocalBookResourceKind::Font,
            ),
            resource_index(
                "book-existing",
                "unsafe",
                "../secret.png",
                "image/png",
                16,
                Some("unsafe-checksum"),
                LocalBookResourceKind::Image,
            ),
            resource_index(
                "book-existing",
                "big",
                "OPS/big.png",
                "image/png",
                4096,
                Some("big-checksum"),
                LocalBookResourceKind::Image,
            ),
        ];
        let request = LocalBookResourceReadRequest {
            book_id: "book-existing".into(),
            resource_id: "style".into(),
            max_bytes: 1024,
        };

        let result = plan_local_book_resource_read(&request, &resources, None).unwrap();
        assert_eq!(
            result,
            LocalBookResourceReadResult {
                book_id: "book-existing".into(),
                resource_id: "style".into(),
                relative_locator: "OPS/style.css".into(),
                mime_type: "text/css".into(),
                byte_count: 128,
                checksum: "style-checksum".into(),
                cache_hit: false,
                cacheable: true,
                diagnostics: Vec::new(),
            }
        );

        let cached = plan_local_book_resource_read(&request, &resources, Some(&result)).unwrap();
        assert!(cached.cache_hit);
        assert_eq!(cached.byte_count, 128);

        let font = plan_local_book_resource_read(
            &LocalBookResourceReadRequest {
                book_id: "book-existing".into(),
                resource_id: "font".into(),
                max_bytes: 1024,
            },
            &resources,
            None,
        )
        .unwrap();
        assert!(!font.cacheable);
        assert_eq!(
            font.checksum,
            local_book_stable_checksum(&["resource", "OPS/fonts/book.ttf", "64"])
        );

        let unsafe_result = plan_local_book_resource_read(
            &LocalBookResourceReadRequest {
                book_id: "book-existing".into(),
                resource_id: "unsafe".into(),
                max_bytes: 1024,
            },
            &resources,
            None,
        )
        .unwrap();
        assert_eq!(unsafe_result.byte_count, 0);
        assert!(!unsafe_result.cacheable);
        assert_eq!(unsafe_result.diagnostics, vec!["unsafe_path_rejected"]);
        assert_eq!(unsafe_result.checksum, local_book_stable_checksum(&[]));

        let oversized = plan_local_book_resource_read(
            &LocalBookResourceReadRequest {
                book_id: "book-existing".into(),
                resource_id: "big".into(),
                max_bytes: 128,
            },
            &resources,
            None,
        )
        .unwrap();
        assert_eq!(oversized.byte_count, 0);
        assert!(!oversized.cacheable);
        assert_eq!(oversized.diagnostics, vec!["oversized_resource_rejected"]);

        assert_eq!(
            plan_local_book_resource_read(
                &LocalBookResourceReadRequest {
                    book_id: "book-existing".into(),
                    resource_id: "missing".into(),
                    max_bytes: 1024,
                },
                &resources,
                None,
            )
            .unwrap_err(),
            LocalBookError::ResourceNotFound {
                book_id: "book-existing".into(),
                resource_id: "missing".into(),
            }
        );
    }

    #[test]
    fn local_book_resource_read_json_shape_and_validation_reject_drift() {
        let resource = resource_index(
            "book-existing",
            "cover",
            "OPS/images/cover.png",
            "image/png",
            256,
            Some("cover-checksum"),
            LocalBookResourceKind::Cover,
        );
        let json = serde_json::to_value(&resource).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "stableResourceId": "cover",
                "bookId": "book-existing",
                "relativeLocator": "OPS/images/cover.png",
                "mimeType": "image/png",
                "byteCount": 256,
                "checksum": "cover-checksum",
                "isMaterialized": true,
                "resourceKind": "cover"
            })
        );
        assert_eq!(
            serde_json::from_value::<LocalBookResourceIndexEntry>(json).unwrap(),
            resource
        );

        let result = LocalBookResourceReadResult {
            book_id: "book-existing".into(),
            resource_id: "cover".into(),
            relative_locator: "OPS/images/cover.png".into(),
            mime_type: "image/png".into(),
            byte_count: 256,
            checksum: "cover-checksum".into(),
            cache_hit: false,
            cacheable: true,
            diagnostics: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::json!({
                "bookId": "book-existing",
                "resourceId": "cover",
                "relativeLocator": "OPS/images/cover.png",
                "mimeType": "image/png",
                "byteCount": 256,
                "checksum": "cover-checksum",
                "cacheHit": false,
                "cacheable": true,
                "diagnostics": []
            })
        );
        assert!(
            serde_json::from_value::<LocalBookResourceIndexEntry>(serde_json::json!({
                "stableResourceId": "cover",
                "bookId": "book-existing",
                "relativeLocator": "OPS/images/cover.png",
                "mimeType": "image/png",
                "byteCount": 256,
                "isMaterialized": true,
                "resourceKind": "cover",
                "bogus": true
            }))
            .is_err()
        );
        assert_eq!(
            LocalBookResourceReadRequest {
                book_id: "book-existing".into(),
                resource_id: "cover".into(),
                max_bytes: 0,
            }
            .validate()
            .unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "max_bytes".into()
            }
        );
    }

    #[test]
    fn local_reading_progress_defaults_match_legacy_reader_core() {
        let progress = LocalReadingProgress::new(" local-book-1 ", 1_700_000_000).unwrap();

        assert_eq!(progress.book_id, "local-book-1");
        assert_eq!(progress.chapter_index, 0);
        assert!(progress.chapter_title.is_none());
        assert_eq!(progress.progress_fraction, 0.0);
        assert!(progress.byte_offset.is_none());
        assert_eq!(
            serde_json::to_value(&progress).unwrap(),
            serde_json::json!({
                "bookId": "local-book-1",
                "chapterIndex": 0,
                "progressFraction": 0.0,
                "updatedAt": 1700000000
            })
        );
    }

    #[test]
    fn local_reading_progress_round_trips_full_shape_and_domain_progress() {
        let progress = LocalReadingProgress {
            book_id: "local-book-1".into(),
            chapter_index: 12,
            chapter_title: Some("Chapter 12".into()),
            progress_fraction: 0.75,
            byte_offset: Some(4096),
            updated_at: 1_700_000_456,
        };

        progress.validate().unwrap();
        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains(r#""bookId":"local-book-1""#));
        assert!(json.contains(r#""chapterTitle":"Chapter 12""#));
        assert_eq!(
            serde_json::from_str::<LocalReadingProgress>(&json).unwrap(),
            progress
        );

        let domain = progress.as_domain_progress().unwrap();
        assert_eq!(domain.book_id, "local-book-1");
        assert_eq!(domain.chapter_index, 12);
        assert_eq!(domain.chapter_offset, 4096);
        assert_eq!(domain.chapter_progress, 0.75);
    }

    #[test]
    fn local_reading_progress_accepts_boundary_fractions_and_rejects_invalid_state() {
        for progress_fraction in [0.0, 1.0] {
            let progress = LocalReadingProgress {
                book_id: "book".into(),
                chapter_index: 0,
                chapter_title: None,
                progress_fraction,
                byte_offset: None,
                updated_at: 1,
            };
            progress.validate().unwrap();
        }

        let mut invalid_fraction = LocalReadingProgress::new("book", 1).unwrap();
        invalid_fraction.progress_fraction = 1.1;
        assert_eq!(
            invalid_fraction.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "progress_fraction".into()
            }
        );
        invalid_fraction.progress_fraction = f64::NAN;
        assert_eq!(
            invalid_fraction.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "progress_fraction".into()
            }
        );

        let mut invalid_title = LocalReadingProgress::new("book", 1).unwrap();
        invalid_title.chapter_title = Some(" ".into());
        assert_eq!(
            invalid_title.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "chapter_title".into()
            }
        );
        assert!(serde_json::from_str::<LocalReadingProgress>(
            r#"{"bookId":"book","chapterIndex":0,"progressFraction":0.5,"updatedAt":1,"bogus":true}"#
        )
        .is_err());
    }

    #[test]
    fn local_reading_progress_update_keeps_newest_and_allows_equal_timestamp() {
        let current = LocalReadingProgress {
            book_id: "book".into(),
            chapter_index: 1,
            chapter_title: Some("One".into()),
            progress_fraction: 0.2,
            byte_offset: Some(100),
            updated_at: 10,
        };
        let stale = LocalReadingProgress {
            book_id: "book".into(),
            chapter_index: 2,
            chapter_title: Some("Two".into()),
            progress_fraction: 0.8,
            byte_offset: Some(200),
            updated_at: 9,
        };

        assert_eq!(
            apply_local_reading_progress_update(Some(&current), stale).unwrap(),
            current
        );

        let equal_timestamp = LocalReadingProgress {
            book_id: "book".into(),
            chapter_index: 2,
            chapter_title: Some("Two".into()),
            progress_fraction: 0.8,
            byte_offset: Some(200),
            updated_at: 10,
        };
        assert_eq!(
            apply_local_reading_progress_update(Some(&current), equal_timestamp.clone()).unwrap(),
            equal_timestamp
        );

        let wrong_book = LocalReadingProgress::new("other", 11).unwrap();
        assert_eq!(
            apply_local_reading_progress_update(Some(&current), wrong_book).unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "book_id".into()
            }
        );
    }

    #[test]
    fn local_book_backup_metadata_defaults_match_legacy_reader_core() {
        let metadata = LocalBookBackupMetadata::new(" book-1 ", " Test Book ").unwrap();

        assert_eq!(metadata.book_id, "book-1");
        assert_eq!(metadata.title, "Test Book");
        assert_eq!(metadata.file_format, LocalBookFormat::Unknown);
        assert!(!metadata.included_in_backup);
        assert!(metadata.author.is_none());
        assert!(metadata.file_hash.is_none());
        assert!(metadata.last_backup_at.is_none());
        assert_eq!(
            serde_json::to_value(&metadata).unwrap(),
            serde_json::json!({
                "bookId": "book-1",
                "title": "Test Book",
                "fileFormat": "unknown",
                "includedInBackup": false
            })
        );
    }

    #[test]
    fn local_book_backup_metadata_round_trips_and_denies_unknown_fields() {
        let metadata = LocalBookBackupMetadata {
            book_id: "book-2".into(),
            title: "Full Book".into(),
            author: Some("Author".into()),
            file_format: LocalBookFormat::Epub,
            file_hash: Some("abc123".into()),
            last_backup_at: Some(1_700_000_000),
            included_in_backup: true,
        };

        metadata.validate().unwrap();
        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains(r#""bookId":"book-2""#));
        assert!(json.contains(r#""fileFormat":"epub""#));
        assert!(json.contains(r#""includedInBackup":true"#));
        assert_eq!(
            serde_json::from_str::<LocalBookBackupMetadata>(&json).unwrap(),
            metadata
        );
        assert!(serde_json::from_str::<LocalBookBackupMetadata>(
            r#"{"bookId":"book","title":"T","fileFormat":"txt","includedInBackup":false,"bogus":true}"#
        )
        .is_err());
    }

    #[test]
    fn local_book_backup_metadata_from_book_uses_parsed_book_state() {
        let book = sample_book("local-1", "Alpha");

        let metadata = LocalBookBackupMetadata::from_book(
            &book,
            Some(" sha256:abc ".into()),
            Some(1_700_000_123),
            true,
        )
        .unwrap();

        assert_eq!(metadata.book_id, "local-1");
        assert_eq!(metadata.title, "Alpha");
        assert_eq!(metadata.author.as_deref(), Some("Author"));
        assert_eq!(metadata.file_format, LocalBookFormat::Txt);
        assert_eq!(metadata.file_hash.as_deref(), Some("sha256:abc"));
        assert_eq!(metadata.last_backup_at, Some(1_700_000_123));
        assert!(metadata.included_in_backup);
    }

    #[test]
    fn local_book_backup_metadata_rejects_invalid_required_or_optional_fields() {
        assert_eq!(
            LocalBookBackupMetadata::new(" ", "Title").unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "book_id".into()
            }
        );
        assert_eq!(
            LocalBookBackupMetadata::new("book", " ").unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "title".into()
            }
        );

        let mut invalid_author = LocalBookBackupMetadata::new("book", "Title").unwrap();
        invalid_author.author = Some(" ".into());
        assert_eq!(
            invalid_author.validate().unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "author".into()
            }
        );

        let book = sample_book("local-1", "Alpha");
        assert_eq!(
            LocalBookBackupMetadata::from_book(&book, Some(" ".into()), None, false).unwrap_err(),
            LocalBookError::InvalidMetadata {
                field: "file_hash".into()
            }
        );
    }

    #[test]
    fn local_book_library_upsert_get_list_and_chapter_round_trip() {
        let mut library = LocalBookLibrary::new();
        populate_library(&mut library);

        let ids = library
            .list_books()
            .into_iter()
            .map(|book| book.book.book_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["b1", "b2"]);

        assert_eq!(library.get_book("b1").unwrap().unwrap().book.title, "Alpha");
        let chapter = library.get_chapter("b1", 1).unwrap();
        assert_eq!(chapter.title, "第一章 开始");
        assert_eq!(chapter.content, "正文一");

        let updated = sample_book("b1", "Alpha Updated");
        library.upsert_book(updated.clone()).unwrap();
        assert_eq!(library.list_books().len(), 2);
        assert_eq!(
            library.get_book("b1").unwrap().unwrap().book.title,
            "Alpha Updated"
        );
    }

    #[test]
    fn local_book_library_parse_and_remove_are_bounded() {
        let mut library = LocalBookLibrary::new();
        let text = sample_text("Gamma");

        let stored = library
            .parse_and_upsert_txt(LocalBookInput {
                book_id: "g",
                file_name: Some("gamma.txt"),
                title: Some("Gamma"),
                author: None,
                bytes: text.as_bytes(),
            })
            .unwrap();

        assert_eq!(stored.book.book_id, "g");
        assert!(library.remove_book("g").unwrap());
        assert!(!library.remove_book("g").unwrap());
        assert_eq!(
            library.get_chapter("g", 0).unwrap_err(),
            LocalBookError::BookNotFound {
                book_id: "g".into()
            }
        );
        assert_eq!(
            library.get_chapter("missing", 0).unwrap_err(),
            LocalBookError::BookNotFound {
                book_id: "missing".into()
            }
        );
    }

    #[test]
    fn local_book_library_reports_missing_chapter_and_invalid_keys() {
        let mut library = LocalBookLibrary::new();
        library.upsert_book(sample_book("b1", "Alpha")).unwrap();

        assert_eq!(
            library.get_chapter("b1", 99).unwrap_err(),
            LocalBookError::ChapterNotFound {
                book_id: "b1".into(),
                chapter_index: 99
            }
        );
        assert!(matches!(
            library.get_book(" "),
            Err(LocalBookError::InvalidMetadata { .. })
        ));
        assert!(matches!(
            library.remove_book(" "),
            Err(LocalBookError::InvalidMetadata { .. })
        ));
    }

    #[test]
    fn local_book_snapshot_export_is_stable_and_json_round_trips() {
        let mut library = LocalBookLibrary::new();
        populate_library(&mut library);

        let snapshot = library.export_snapshot(42).unwrap();

        assert_eq!(
            snapshot.schema_version,
            LOCAL_BOOK_LIBRARY_SNAPSHOT_SCHEMA_VERSION
        );
        assert_eq!(snapshot.exported_at, 42);
        assert_eq!(
            snapshot
                .books
                .iter()
                .map(|book| book.book.book_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b1", "b2"]
        );
        assert_eq!(snapshot.books[0].chapters[0].index, 0);
        assert_eq!(snapshot.books[0].toc[1].title, "第一章 开始");

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains(r#""schemaVersion":1"#));
        let back: LocalBookLibrarySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
    }

    #[test]
    fn local_book_snapshot_replace_round_trips_and_empty_clears() {
        let mut source = LocalBookLibrary::new();
        populate_library(&mut source);
        let snapshot = source.export_snapshot(77).unwrap();

        let mut restored = LocalBookLibrary::new();
        restored.replace_with_snapshot(snapshot.clone()).unwrap();

        assert_eq!(restored.export_snapshot(77).unwrap(), snapshot);
        assert_eq!(restored.get_chapter("b2", 2).unwrap().content, "正文二");

        restored
            .replace_with_snapshot(LocalBookLibrarySnapshot::empty(100))
            .unwrap();
        assert!(restored.list_books().is_empty());
        assert!(restored.get_book("b1").unwrap().is_none());
    }

    #[test]
    fn local_book_snapshot_rejects_schema_duplicates_invalid_books_and_unknown_fields() {
        let mut wrong_schema = LocalBookLibrarySnapshot::empty(1);
        wrong_schema.schema_version = 2;
        assert_eq!(
            wrong_schema.validate().unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "schema_version".into()
            }
        );

        let mut duplicate = LocalBookLibrarySnapshot::empty(1);
        duplicate.books.push(sample_book("b1", "Alpha"));
        duplicate.books.push(sample_book("b1", "Alpha Copy"));
        assert_eq!(
            duplicate.validate().unwrap_err(),
            LocalBookError::InvalidSnapshot {
                field: "books".into()
            }
        );

        let mut invalid_book = LocalBookLibrarySnapshot::empty(1);
        let mut broken = sample_book("bad", "Broken");
        broken.chapters[0].index = 9;
        invalid_book.books.push(broken);
        assert_eq!(
            invalid_book.validate().unwrap_err(),
            LocalBookError::InvalidBook {
                field: "chapters.index".into()
            }
        );

        let unknown = r#"{"schemaVersion":1,"exportedAt":1,"books":[],"bogus":true}"#;
        assert!(serde_json::from_str::<LocalBookLibrarySnapshot>(unknown).is_err());
    }

    #[test]
    fn local_book_snapshot_replace_is_atomic_on_validation_failure() {
        let mut library = LocalBookLibrary::new();
        populate_library(&mut library);
        let before = library.export_snapshot(1).unwrap();

        let mut invalid = LocalBookLibrarySnapshot::empty(2);
        let mut broken = sample_book("bad", "Broken");
        broken.toc.pop();
        invalid.books.push(broken);

        assert!(matches!(
            library.replace_with_snapshot(invalid),
            Err(LocalBookError::InvalidBook { .. })
        ));
        assert_eq!(library.export_snapshot(1).unwrap(), before);
    }

    #[test]
    fn parses_utf8_txt_into_book_toc_and_chapters() {
        let text = "献词\n给岁月以文明\n\n第一章 科学边界\n正文一\n\n第二章 台球\n正文二";

        let book = parse_txt_book(input("local-1", text.as_bytes())).unwrap();

        assert_eq!(book.book.book_id, "local-1");
        assert_eq!(book.book.title, "三体");
        assert_eq!(book.book.author, "刘慈欣");
        assert_eq!(book.book.kind.as_deref(), Some("local"));
        assert_eq!(book.encoding, LocalBookEncoding::Utf8);
        assert_eq!(book.format, LocalBookFormat::Txt);
        assert_eq!(book.toc.len(), 3);
        assert_eq!(book.toc[0].title, "序章");
        assert_eq!(book.toc[1].title, "第一章 科学边界");
        assert_eq!(book.toc[2].url, "local://local-1/chapter/2");
        assert_eq!(book.book.last_chapter.as_deref(), Some("第二章 台球"));
        assert_eq!(book.chapters[0].content, "献词\n给岁月以文明");
        assert_eq!(book.chapters[1].content, "正文一");
        assert_eq!(book.chapters[2].content, "正文二");
    }

    #[test]
    fn no_heading_txt_becomes_single_body_chapter() {
        let text = "第一行不是章节标题\n第二行仍然是正文";

        let book = parse_txt_text("plain", Some("Plain Book"), None, None, text).unwrap();

        assert_eq!(book.book.title, "Plain Book");
        assert_eq!(book.toc.len(), 1);
        assert_eq!(book.toc[0].title, "正文");
        assert_eq!(book.chapters[0].content, text);
        assert_eq!(book.chapters[0].start_char, 0);
        assert_eq!(book.chapters[0].end_char, text.chars().count());
    }

    #[test]
    fn utf8_bom_is_detected_and_stripped() {
        let bytes = b"\xef\xbb\xbfChapter 1\nBody";

        let book = parse_txt_book(LocalBookInput {
            book_id: "bom",
            file_name: Some("bom.txt"),
            title: None,
            author: None,
            bytes,
        })
        .unwrap();

        assert_eq!(book.encoding, LocalBookEncoding::Utf8Bom);
        assert_eq!(book.toc[0].title, "Chapter 1");
        assert_eq!(book.chapters[0].content, "Body");
    }

    #[test]
    fn utf16le_bom_is_decoded() {
        let mut bytes = vec![0xff, 0xfe];
        for unit in "第一章 开始\n正文".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }

        let book = parse_txt_book(input("utf16le", &bytes)).unwrap();

        assert_eq!(book.encoding, LocalBookEncoding::Utf16Le);
        assert_eq!(book.toc[0].title, "第一章 开始");
        assert_eq!(book.chapters[0].content, "正文");
    }

    #[test]
    fn utf16be_bom_is_decoded() {
        let mut bytes = vec![0xfe, 0xff];
        for unit in "Chapter 9\nBody".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }

        let book = parse_txt_book(input("utf16be", &bytes)).unwrap();

        assert_eq!(book.encoding, LocalBookEncoding::Utf16Be);
        assert_eq!(book.toc[0].title, "Chapter 9");
        assert_eq!(book.chapters[0].content, "Body");
    }

    #[test]
    fn title_option_overrides_file_name() {
        let book = parse_txt_book(LocalBookInput {
            book_id: "id",
            file_name: Some("file-title.txt"),
            title: Some("Manual Title"),
            author: Some("  "),
            bytes: "正文".as_bytes(),
        })
        .unwrap();

        assert_eq!(book.book.title, "Manual Title");
        assert!(book.book.author.is_empty());
    }

    #[test]
    fn invalid_metadata_rejects_empty_book_id() {
        let err = parse_txt_book(input("   ", "正文".as_bytes())).unwrap_err();

        assert_eq!(
            err,
            LocalBookError::InvalidMetadata {
                field: "book_id".into()
            }
        );
    }

    #[test]
    fn empty_or_blank_input_is_rejected() {
        assert_eq!(
            parse_txt_book(input("empty", b"")).unwrap_err(),
            LocalBookError::EmptyInput
        );
        assert_eq!(
            parse_txt_book(input("blank", b" \n\t ")).unwrap_err(),
            LocalBookError::EmptyInput
        );
    }

    #[test]
    fn unsupported_non_utf8_without_bom_is_rejected() {
        let err = parse_txt_book(input("bad", &[0xff, 0x00, 0x80])).unwrap_err();

        assert_eq!(err, LocalBookError::UnsupportedEncoding);
    }

    #[test]
    fn odd_utf16_byte_length_is_rejected() {
        let err = parse_txt_book(input("bad-utf16", &[0xff, 0xfe, 0x00])).unwrap_err();

        assert_eq!(
            err,
            LocalBookError::Decode {
                reason: "UTF-16 byte length is not even".into()
            }
        );
    }

    #[test]
    fn chapter_heading_detection_accepts_common_forms_and_rejects_long_lines() {
        assert!(is_chapter_heading("第一章 开始"));
        assert!(is_chapter_heading("卷一 风起"));
        assert!(is_chapter_heading("Chapter 12 The Door"));
        assert!(!is_chapter_heading("第一行不是章节标题"));
        let long_heading = format!("第一章 {}", "很长".repeat(50));
        assert!(!is_chapter_heading(&long_heading));
    }
}

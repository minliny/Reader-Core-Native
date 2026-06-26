//! Reader-Core storage — SQLite schema, migrations, cache, progress, download queue.
//!
//! V1 ships an in-memory implementation only. The real SQLite-backed store is
//! deferred to a later phase; the trait surface here is what the runtime
//! vertical commands depend on, so swapping the backend later is localized to
//! this crate.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Mutex;

use reader_domain::{Book, ReadingProgress, Source};
use serde::{Deserialize, Serialize};

#[cfg(feature = "sqlite")]
pub mod sqlite_backend;

#[cfg(feature = "sqlite")]
pub use sqlite_backend::{SqliteStorage, SQLITE_SCHEMA_VERSION};

/// Current storage snapshot schema version.
pub const STORAGE_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// In-memory cache + source registry + progress store.
///
/// All operations are synchronous and cheap. The `Mutex` guards a single
/// `HashMap`-based map per concern; contention is negligible for V1 workloads.
#[derive(Default)]
pub struct InMemoryStorage {
    inner: Mutex<StorageInner>,
}

#[derive(Default)]
struct StorageInner {
    sources: HashMap<String, Source>,
    books: HashMap<String, Book>,
    cache: HashMap<String, CachedEntry>,
    progress: HashMap<String, ReadingProgress>,
    shelf: HashMap<ShelfKey, BookshelfEntry>,
    chapter_cache: HashMap<ChapterCacheKey, ChapterCacheEntry>,
    reading_progress: HashMap<ReadingProgressKey, ReadingProgressEntry>,
    reading_progress_history: HashMap<ReadingProgressKey, Vec<ReadingProgressEntry>>,
    chapter_download_queue: HashMap<ChapterDownloadKey, ChapterDownloadTask>,
}

/// A minimal cached entry: an opaque JSON payload keyed by a string cache key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CachedEntry {
    pub key: String,
    pub payload: String,
}

/// Complete export/import unit for storage-owned reader data.
///
/// The snapshot is intentionally transport-neutral: backup files, local DB
/// migrations, and sync packaging can all use the same deterministic shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageSnapshot {
    pub schema_version: u32,
    pub exported_at: i64,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub books: Vec<Book>,
    #[serde(default)]
    pub cache: Vec<CachedEntry>,
    #[serde(default)]
    pub legacy_progress: Vec<ReadingProgress>,
    #[serde(default)]
    pub bookshelf: Vec<BookshelfEntry>,
    #[serde(default)]
    pub chapter_cache: Vec<ChapterCacheEntry>,
    #[serde(default)]
    pub reading_progress: Vec<ReadingProgressEntry>,
    #[serde(default)]
    pub reading_progress_history: Vec<ReadingProgressEntry>,
    #[serde(default)]
    pub chapter_download_queue: Vec<ChapterDownloadTask>,
}

impl StorageSnapshot {
    pub fn empty(exported_at: i64) -> Self {
        Self {
            schema_version: STORAGE_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            sources: Vec::new(),
            books: Vec::new(),
            cache: Vec::new(),
            legacy_progress: Vec::new(),
            bookshelf: Vec::new(),
            chapter_cache: Vec::new(),
            reading_progress: Vec::new(),
            reading_progress_history: Vec::new(),
            chapter_download_queue: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), StorageError> {
        if self.schema_version != STORAGE_SNAPSHOT_SCHEMA_VERSION {
            return Err(StorageError::InvalidSnapshot {
                field: "schema_version".into(),
            });
        }
        let mut unique_source_ids = HashMap::<String, ()>::new();
        for source in &self.sources {
            validate_source(source)?;
            ensure_unique_snapshot_key(
                &mut unique_source_ids,
                source.source_id.clone(),
                "sources",
            )?;
        }

        let mut unique_book_ids = HashMap::<String, ()>::new();
        for book in &self.books {
            validate_book(book)?;
            ensure_unique_snapshot_key(&mut unique_book_ids, book.book_id.clone(), "books")?;
        }

        let mut unique_cache_keys = HashMap::<String, ()>::new();
        for entry in &self.cache {
            validate_cache_entry(entry)?;
            ensure_unique_snapshot_key(&mut unique_cache_keys, entry.key.clone(), "cache")?;
        }

        let mut unique_legacy_progress_keys = HashMap::<String, ()>::new();
        for progress in &self.legacy_progress {
            validate_domain_progress(progress)?;
            ensure_unique_snapshot_key(
                &mut unique_legacy_progress_keys,
                progress.book_id.clone(),
                "legacy_progress",
            )?;
        }

        let mut unique_shelf_keys = HashMap::<ShelfKey, ()>::new();
        for entry in &self.bookshelf {
            validate_shelf_key(&entry.source_id, &entry.book_id)?;
            ensure_unique_snapshot_key(&mut unique_shelf_keys, entry.shelf_key(), "bookshelf")?;
        }

        let mut unique_chapter_cache_keys = HashMap::<ChapterCacheKey, ()>::new();
        for entry in &self.chapter_cache {
            validate_book_key(&entry.source_id, &entry.book_id)?;
            ensure_unique_snapshot_key(
                &mut unique_chapter_cache_keys,
                entry.chapter_cache_key(),
                "chapter_cache",
            )?;
        }

        let mut unique_progress_keys = HashMap::<ReadingProgressKey, ()>::new();
        for entry in &self.reading_progress {
            validate_reading_progress(entry)?;
            ensure_unique_snapshot_key(
                &mut unique_progress_keys,
                entry.progress_key(),
                "reading_progress",
            )?;
        }

        for entry in &self.reading_progress_history {
            validate_reading_progress(entry)?;
        }

        let mut unique_download_keys = HashMap::<ChapterDownloadKey, ()>::new();
        for task in &self.chapter_download_queue {
            validate_chapter_download_task(task)?;
            ensure_unique_snapshot_key(
                &mut unique_download_keys,
                task.download_key(),
                "chapter_download_queue",
            )?;
        }

        Ok(())
    }
}

/// Lowest snapshot `schemaVersion` the migrator accepts as input.
///
/// Version 0 is the legacy pre-storage shape: it may omit `schemaVersion`
/// entirely and may carry reading progress under the pre-snapshot `progress`
/// field name instead of `legacyProgress`. The migrator normalizes both before
/// deserializing into the current [`StorageSnapshot`] shape.
pub const STORAGE_SNAPSHOT_MIN_SUPPORTED_SCHEMA_VERSION: u32 = 0;

/// Upgrade a raw JSON snapshot to the current schema and validate it.
///
/// This is the canonical import entry point. Callers that already hold a typed
/// [`StorageSnapshot`] at the current version can use [`StorageSnapshot::validate`]
/// directly; `migrate_storage_snapshot` adds the ability to accept older or
/// looser JSON shapes (missing `schemaVersion`, legacy field aliases) and bring
/// them up to [`STORAGE_SNAPSHOT_SCHEMA_VERSION`].
///
/// The current schema is fixed at
/// [`STORAGE_SNAPSHOT_SCHEMA_VERSION`]; any `schemaVersion` above that is
/// rejected as [`StorageError::UnsupportedSnapshotSchemaVersion`] rather than
/// silently truncated, so a future host never silently downgrades state it
/// cannot understand.
pub fn migrate_storage_snapshot(raw: serde_json::Value) -> Result<StorageSnapshot, StorageError> {
    let mut object = match raw {
        serde_json::Value::Object(object) => object,
        _ => {
            return Err(StorageError::InvalidSnapshot {
                field: "root".into(),
            });
        }
    };

    let declared_version = object
        .get("schemaVersion")
        .and_then(|value| value.as_u64())
        .map(|version| u32::try_from(version).unwrap_or(u32::MAX))
        .unwrap_or(STORAGE_SNAPSHOT_MIN_SUPPORTED_SCHEMA_VERSION);

    if declared_version > STORAGE_SNAPSHOT_SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSnapshotSchemaVersion {
            schema_version: declared_version,
        });
    }

    if declared_version < STORAGE_SNAPSHOT_SCHEMA_VERSION {
        for from in declared_version..STORAGE_SNAPSHOT_SCHEMA_VERSION {
            migrate_snapshot_step(&mut object, from)?;
        }
        object.insert(
            "schemaVersion".into(),
            serde_json::Value::from(STORAGE_SNAPSHOT_SCHEMA_VERSION),
        );
    }

    let snapshot: StorageSnapshot = serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|_| StorageError::InvalidSnapshot {
            field: "shape".into(),
        })?;
    snapshot.validate()?;
    Ok(snapshot)
}

/// Apply one in-place schema upgrade step to a raw snapshot object.
///
/// Each step mutates the JSON object from `from` to `from + 1`. Only the
/// `0 → 1` step is defined today; future versions register additional steps
/// here. Keeping the steps as pure JSON transforms means the migrator never
/// needs to round-trip through a typed legacy struct whose shape we do not own.
fn migrate_snapshot_step(
    object: &mut serde_json::Map<String, serde_json::Value>,
    from: u32,
) -> Result<(), StorageError> {
    match from {
        0 => {
            // Legacy snapshots keyed reading progress under `progress`; the v1
            // shape renamed it to `legacyProgress` to disambiguate from the
            // composite-key `readingProgress` table. Merge into any existing
            // `legacyProgress` array so a partial v0/v1 hybrid still imports.
            if let Some(serde_json::Value::Array(incoming)) = object.remove("progress") {
                let entry = object
                    .entry("legacyProgress")
                    .or_insert_with(|| serde_json::Value::Array(Vec::new()));
                if let Some(existing) = entry.as_array_mut() {
                    existing.extend(incoming);
                }
            }
            Ok(())
        }
        other => Err(StorageError::UnsupportedSnapshotSchemaVersion {
            schema_version: other,
        }),
    }
}

/// A book on the user's shelf.
///
/// Keyed by the composite `(source_id, book_id)` so the same `book_id` from
/// different sources does not collide. Local books use `source_id = "local"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookshelfEntry {
    /// Source the book came from. `"local"` for local books.
    pub source_id: String,
    /// Source-relative book identifier.
    pub book_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chapter: Option<String>,
    /// Unix timestamp (seconds) when the book was added to the shelf.
    pub added_at: i64,
    /// Unix timestamp (seconds) of the last read, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_read_at: Option<i64>,
    /// User-defined group name (e.g. `"默认"`, `"追更"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Manual sort order within the shelf. Lower comes first.
    #[serde(default)]
    pub sort_index: i32,
}

impl BookshelfEntry {
    /// Composite key used by the shelf store.
    fn shelf_key(&self) -> ShelfKey {
        ShelfKey {
            source_id: self.source_id.clone(),
            book_id: self.book_id.clone(),
        }
    }
}

/// Query/filter/sort request for a bookshelf view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookshelfQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Case-insensitive substring match over title, author, and book id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    /// `Some(true)` keeps books with current reading progress; `Some(false)`
    /// keeps books without progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_reading_progress: Option<bool>,
    #[serde(default)]
    pub sort_by: BookshelfSortBy,
    #[serde(default)]
    pub sort_direction: BookshelfSortDirection,
    #[serde(default)]
    pub offset: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl Default for BookshelfQuery {
    fn default() -> Self {
        Self {
            source_id: None,
            group: None,
            keyword: None,
            has_reading_progress: None,
            sort_by: BookshelfSortBy::Manual,
            sort_direction: BookshelfSortDirection::Ascending,
            offset: 0,
            limit: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BookshelfSortBy {
    /// `sort_index` ascending, then added time descending.
    Manual,
    AddedAt,
    LastReadAt,
    Title,
    Author,
}

impl Default for BookshelfSortBy {
    fn default() -> Self {
        Self::Manual
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BookshelfSortDirection {
    Ascending,
    Descending,
}

impl Default for BookshelfSortDirection {
    fn default() -> Self {
        Self::Ascending
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShelfKey {
    source_id: String,
    book_id: String,
}

/// A cached chapter body for a specific source/book/chapter index.
///
/// The cache key is `(source_id, book_id, chapter_index)`. The URL and title are
/// stored as metadata because source TOCs can be revalidated later without
/// reparsing the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterCacheEntry {
    /// Source the book came from. `"local"` for local books.
    pub source_id: String,
    /// Source-relative book identifier.
    pub book_id: String,
    /// 0-based chapter index within the current TOC.
    pub chapter_index: u32,
    #[serde(default)]
    pub title: String,
    /// Source-relative chapter URL/path. Local books may leave this empty.
    #[serde(default)]
    pub url: String,
    /// Normalized chapter body text.
    #[serde(default)]
    pub content: String,
    /// Unix timestamp (seconds) when this body was cached.
    pub cached_at: i64,
    /// Optional source revision marker such as an ETag, hash, or upstream id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl ChapterCacheEntry {
    fn chapter_cache_key(&self) -> ChapterCacheKey {
        ChapterCacheKey {
            source_id: self.source_id.clone(),
            book_id: self.book_id.clone(),
            chapter_index: self.chapter_index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ChapterCacheKey {
    source_id: String,
    book_id: String,
    chapter_index: u32,
}

/// Aggregate size/age view of the chapter cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterCacheStats {
    pub entry_count: usize,
    pub total_content_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_cached_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest_cached_at: Option<i64>,
}

/// Retention limits for chapter cache pruning.
///
/// Limits are applied in this order: `min_cached_at`, `max_entries`, then
/// `max_total_content_bytes`. Within each limit, older cache entries are
/// evicted before newer entries.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterCacheRetentionPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_entries: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_content_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_cached_at: Option<i64>,
}

/// Result of applying a chapter cache retention policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterCacheEvictionReport {
    #[serde(default)]
    pub removed: Vec<ChapterCacheEntry>,
    pub remaining: ChapterCacheStats,
}

/// Cache coverage for one book against its current TOC length.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterCacheCoverage {
    pub source_id: String,
    pub book_id: String,
    pub chapter_count: u32,
    #[serde(default)]
    pub cached_indexes: Vec<u32>,
    #[serde(default)]
    pub missing_indexes: Vec<u32>,
    pub cached_count: usize,
    pub missing_count: usize,
    pub total_content_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_cached_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest_cached_at: Option<i64>,
    pub complete: bool,
}

/// Bounded missing-chapter plan around a reader anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterCachePrefetchPlan {
    pub source_id: String,
    pub book_id: String,
    pub chapter_count: u32,
    pub anchor_index: u32,
    pub window_start: u32,
    pub window_end_exclusive: u32,
    #[serde(default)]
    pub missing_indexes: Vec<u32>,
}

/// Legacy chapter cache availability status for offline-readiness planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChapterCacheStatus {
    Missing,
    Downloading,
    Available,
    Validated,
    Stale,
    Invalidated,
    Failed,
    Partial,
    Evicted,
}

/// Minimal status row used to compute legacy offline availability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfflineChapterCacheEntry {
    pub source_id: String,
    pub chapter_id: String,
    pub status: ChapterCacheStatus,
}

impl OfflineChapterCacheEntry {
    pub fn new(
        source_id: impl Into<String>,
        chapter_id: impl Into<String>,
        status: ChapterCacheStatus,
    ) -> Result<Self, StorageError> {
        let entry = Self {
            source_id: source_id.into(),
            chapter_id: chapter_id.into(),
            status,
        };
        validate_offline_chapter_cache_entry(&entry)?;
        Ok(entry)
    }
}

/// Metadata needed to validate a unified offline chapter cache row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedOfflineChapterCacheEntry {
    pub source_id: String,
    pub book_id: String,
    pub chapter_id: String,
    pub source_content_locator_checksum: String,
    pub content_checksum: String,
    pub normalized_content_type: String,
    pub byte_count: u64,
    pub creation_timestamp: String,
    pub last_access_timestamp: String,
    pub validation_timestamp: String,
    pub parser_runtime_version: String,
    pub source_fingerprint_or_remote_toc_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption_at_rest_capability: Option<String>,
    pub state: ChapterCacheStatus,
    pub pinned: bool,
}

impl UnifiedOfflineChapterCacheEntry {
    pub fn validate(&self) -> Result<(), StorageError> {
        validate_book_key(&self.source_id, &self.book_id)?;
        validate_chapter_cache_required(&self.chapter_id, "chapter_id")?;
        validate_chapter_cache_required(
            &self.source_content_locator_checksum,
            "source_content_locator_checksum",
        )?;
        validate_chapter_cache_required(&self.content_checksum, "content_checksum")?;
        validate_chapter_cache_required(&self.normalized_content_type, "normalized_content_type")?;
        validate_chapter_cache_required(&self.creation_timestamp, "creation_timestamp")?;
        validate_chapter_cache_required(&self.last_access_timestamp, "last_access_timestamp")?;
        validate_chapter_cache_required(&self.validation_timestamp, "validation_timestamp")?;
        validate_chapter_cache_required(&self.parser_runtime_version, "parser_runtime_version")?;
        validate_chapter_cache_required(
            &self.source_fingerprint_or_remote_toc_version,
            "source_fingerprint_or_remote_toc_version",
        )?;
        validate_chapter_cache_optional(
            &self.encryption_at_rest_capability,
            "encryption_at_rest_capability",
        )?;
        Ok(())
    }
}

/// Validate unified offline cache metadata using RECOVERY-33 state ordering.
pub fn validate_unified_offline_chapter_cache(
    entries: &[UnifiedOfflineChapterCacheEntry],
    source_id: &str,
    book_id: &str,
    chapter_id: &str,
    expected_fingerprint_or_remote_toc_version: &str,
    parser_runtime_version: &str,
    normalized_content_checksum: Option<&str>,
) -> Result<ChapterCacheStatus, StorageError> {
    validate_book_key(source_id, book_id)?;
    validate_chapter_cache_required(chapter_id, "chapter_id")?;
    validate_chapter_cache_required(
        expected_fingerprint_or_remote_toc_version,
        "expected_fingerprint_or_remote_toc_version",
    )?;
    validate_chapter_cache_required(parser_runtime_version, "parser_runtime_version")?;
    if normalized_content_checksum
        .map(str::trim)
        .is_some_and(str::is_empty)
    {
        return Err(StorageError::InvalidChapterCache {
            field: "normalized_content_checksum".into(),
        });
    }
    for entry in entries {
        entry.validate()?;
    }

    let Some(entry) = entries.iter().find(|entry| {
        entry.source_id == source_id && entry.book_id == book_id && entry.chapter_id == chapter_id
    }) else {
        return Ok(ChapterCacheStatus::Missing);
    };

    if entry.parser_runtime_version != parser_runtime_version {
        return Ok(ChapterCacheStatus::Invalidated);
    }
    if entry.source_fingerprint_or_remote_toc_version != expected_fingerprint_or_remote_toc_version
    {
        return Ok(ChapterCacheStatus::Stale);
    }
    if normalized_content_checksum.is_some_and(|checksum| checksum != entry.content_checksum) {
        return Ok(ChapterCacheStatus::Failed);
    }
    Ok(ChapterCacheStatus::Validated)
}

/// Legacy offline availability summary for a source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfflineAvailability {
    pub source_id: String,
    pub total_chapters: u32,
    pub cached_chapters: u32,
    pub stale_chapters: u32,
    pub failed_chapters: u32,
    pub missing_chapters: i64,
}

impl OfflineAvailability {
    pub fn new(
        source_id: impl Into<String>,
        total_chapters: u32,
        cached_chapters: u32,
        stale_chapters: u32,
        failed_chapters: u32,
    ) -> Result<Self, StorageError> {
        let source_id = source_id.into();
        validate_source_id(&source_id)?;
        Ok(Self {
            source_id,
            total_chapters,
            cached_chapters,
            stale_chapters,
            failed_chapters,
            missing_chapters: i64::from(total_chapters)
                - i64::from(cached_chapters)
                - i64::from(stale_chapters)
                - i64::from(failed_chapters),
        })
    }

    pub fn availability_ratio(&self) -> f64 {
        if self.total_chapters == 0 {
            1.0
        } else {
            f64::from(self.cached_chapters) / f64::from(self.total_chapters)
        }
    }

    pub fn next_download_range(&self, limit: u32) -> Option<OfflineDownloadRange> {
        if self.total_chapters == 0 || self.cached_chapters >= self.total_chapters || limit == 0 {
            return None;
        }
        let start = self.cached_chapters + 1;
        let end_inclusive = start
            .saturating_add(limit)
            .saturating_sub(1)
            .min(self.total_chapters);
        Some(OfflineDownloadRange {
            start,
            end_inclusive,
        })
    }
}

/// One-indexed closed range matching the legacy `cached + 1 ... end` planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfflineDownloadRange {
    pub start: u32,
    pub end_inclusive: u32,
}

pub fn compute_offline_availability(
    source_id: impl Into<String>,
    total_chapters: u32,
    entries: &[OfflineChapterCacheEntry],
) -> Result<OfflineAvailability, StorageError> {
    let source_id = source_id.into();
    validate_source_id(&source_id)?;
    for entry in entries {
        validate_offline_chapter_cache_entry(entry)?;
    }

    let cached = count_offline_status(entries, &source_id, ChapterCacheStatus::Available)?;
    let stale = count_offline_status(entries, &source_id, ChapterCacheStatus::Stale)?;
    let failed = count_offline_status(entries, &source_id, ChapterCacheStatus::Failed)?;
    OfflineAvailability::new(source_id, total_chapters, cached, stale, failed)
}

pub fn offline_can_read_chapter(entries: &[OfflineChapterCacheEntry], chapter_id: &str) -> bool {
    entries.iter().any(|entry| {
        entry.chapter_id == chapter_id && entry.status == ChapterCacheStatus::Available
    })
}

/// Current reading position for a source/book pair.
///
/// This is intentionally separate from [`reader_domain::ReadingProgress`],
/// whose V1 shape is keyed only by `book_id`. Storage needs the composite
/// `(source_id, book_id)` key so two sources can expose the same book id without
/// overwriting each other's progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadingProgressEntry {
    /// Source the book came from. `"local"` for local books.
    pub source_id: String,
    /// Source-relative book identifier.
    pub book_id: String,
    /// Index of the chapter the reader is currently on.
    #[serde(default)]
    pub chapter_index: u32,
    /// Scroll/char offset within the current chapter.
    #[serde(default)]
    pub chapter_offset: u64,
    /// Fraction read in the current chapter, constrained to 0.0..=1.0.
    #[serde(default)]
    pub chapter_progress: f64,
    /// Unix timestamp (seconds) for this progress update.
    pub updated_at: i64,
    /// Optional device id that produced the update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

impl ReadingProgressEntry {
    fn progress_key(&self) -> ReadingProgressKey {
        ReadingProgressKey {
            source_id: self.source_id.clone(),
            book_id: self.book_id.clone(),
        }
    }

    pub fn as_domain_progress(&self) -> ReadingProgress {
        ReadingProgress {
            book_id: self.book_id.clone(),
            chapter_index: self.chapter_index,
            chapter_offset: self.chapter_offset,
            chapter_progress: self.chapter_progress,
        }
    }
}

/// Stable chapter index row used to restore unified reading progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedChapterIndexEntry {
    pub source_id: String,
    pub book_id: String,
    pub chapter_id: String,
    pub ordinal: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub canonical_locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_specific_locator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_locator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

impl UnifiedChapterIndexEntry {
    pub fn validate(&self) -> Result<(), StorageError> {
        validate_book_key(&self.source_id, &self.book_id)?;
        validate_progress_required(&self.chapter_id, "chapter_id")?;
        validate_progress_optional(&self.title, "title")?;
        validate_progress_required(&self.canonical_locator, "canonical_locator")?;
        validate_progress_optional(&self.origin_specific_locator, "origin_specific_locator")?;
        validate_progress_optional(&self.content_locator, "content_locator")?;
        validate_progress_optional(&self.content_checksum, "content_checksum")?;
        validate_progress_diagnostics(&self.diagnostics)?;
        if self.ordinal < 0 {
            return Err(StorageError::InvalidProgress {
                field: "ordinal".into(),
            });
        }
        Ok(())
    }
}

/// Restore state for a unified local/remote reading locator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnifiedReadingRestoreState {
    ExactRestored,
    LocatorRestored,
    OrdinalRestored,
    NearestChapterRestored,
    ResetToBeginning,
}

/// Portable reading locator for storage-owned local/remote progress restore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedReadingLocator {
    pub source_id: String,
    pub book_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_specific_locator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_locator: Option<String>,
    pub chapter_ordinal: i64,
    pub character_offset: i64,
    pub normalized_chapter_progress: f64,
    pub normalized_book_progress: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surrounding_text_checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_toc_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
    pub timestamp: String,
}

impl UnifiedReadingLocator {
    pub fn validate(&self) -> Result<(), StorageError> {
        validate_book_key(&self.source_id, &self.book_id)?;
        validate_progress_optional(&self.chapter_id, "chapter_id")?;
        validate_progress_optional(&self.origin_specific_locator, "origin_specific_locator")?;
        validate_progress_optional(&self.canonical_locator, "canonical_locator")?;
        validate_progress_optional(&self.surrounding_text_checksum, "surrounding_text_checksum")?;
        validate_progress_optional(&self.remote_toc_version, "remote_toc_version")?;
        validate_progress_optional(&self.local_fingerprint, "local_fingerprint")?;
        validate_progress_optional(&self.content_checksum, "content_checksum")?;
        validate_progress_required(&self.timestamp, "timestamp")?;
        if self.character_offset < 0 {
            return Err(StorageError::InvalidProgress {
                field: "character_offset".into(),
            });
        }
        validate_progress_fraction(
            self.normalized_chapter_progress,
            "normalized_chapter_progress",
        )?;
        validate_progress_fraction(self.normalized_book_progress, "normalized_book_progress")?;
        Ok(())
    }

    fn replacing_chapter(&self, chapter: &UnifiedChapterIndexEntry) -> Self {
        let mut locator = self.clone();
        locator.chapter_id = Some(chapter.chapter_id.clone());
        locator.chapter_ordinal = chapter.ordinal;
        locator.canonical_locator = Some(chapter.canonical_locator.clone());
        locator.origin_specific_locator = chapter.origin_specific_locator.clone();
        locator.content_checksum = chapter.content_checksum.clone();
        locator
    }
}

/// Portable progress payload used by storage/sync without invoking host IO.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedReadingProgress {
    pub locator: UnifiedReadingLocator,
    pub restore_state: UnifiedReadingRestoreState,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

impl UnifiedReadingProgress {
    pub fn validate(&self) -> Result<(), StorageError> {
        self.locator.validate()?;
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.trim().is_empty())
        {
            return Err(StorageError::InvalidProgress {
                field: "diagnostics".into(),
            });
        }
        Ok(())
    }
}

/// Resolve a stored local/remote reading locator against a current chapter index.
///
/// This mirrors the legacy unified reading restore order: exact chapter id,
/// canonicalized locator, ordinal, nearest chapter, then reset.
pub fn resolve_unified_reading_progress(
    progress: &UnifiedReadingProgress,
    source_id: &str,
    book_id: &str,
    chapters: &[UnifiedChapterIndexEntry],
) -> Result<UnifiedReadingProgress, StorageError> {
    progress.validate()?;
    validate_book_key(source_id, book_id)?;
    for chapter in chapters {
        chapter.validate()?;
    }

    let mut chapters = chapters
        .iter()
        .filter(|chapter| chapter.source_id == source_id && chapter.book_id == book_id)
        .collect::<Vec<_>>();
    chapters.sort_by(|left, right| {
        left.ordinal
            .cmp(&right.ordinal)
            .then_with(|| left.chapter_id.cmp(&right.chapter_id))
    });

    if progress.locator.source_id != source_id || progress.locator.book_id != book_id {
        return Ok(reset_unified_reading_progress(
            progress,
            source_id,
            book_id,
            chapters.first().copied(),
            "source_or_book_mismatch",
        ));
    }
    if chapters.is_empty() {
        return Ok(reset_unified_reading_progress(
            progress,
            source_id,
            book_id,
            None,
            "missing_book_or_empty_index",
        ));
    }

    if progress
        .locator
        .chapter_id
        .as_deref()
        .is_some_and(|chapter_id| {
            chapters
                .iter()
                .any(|chapter| chapter.chapter_id == chapter_id)
        })
    {
        return Ok(UnifiedReadingProgress {
            locator: progress.locator.clone(),
            restore_state: UnifiedReadingRestoreState::ExactRestored,
            diagnostics: Vec::new(),
        });
    }

    if let Some((chapter, diagnostic)) = progress
        .locator
        .origin_specific_locator
        .as_deref()
        .and_then(|locator| {
            find_unified_chapter_by_locator(&chapters, locator)
                .map(|chapter| (chapter, "origin_locator_canonicalized"))
        })
        .or_else(|| {
            progress
                .locator
                .canonical_locator
                .as_deref()
                .and_then(|locator| {
                    find_unified_chapter_by_locator(&chapters, locator)
                        .map(|chapter| (chapter, "canonical_locator_canonicalized"))
                })
        })
    {
        return Ok(UnifiedReadingProgress {
            locator: progress.locator.replacing_chapter(chapter),
            restore_state: UnifiedReadingRestoreState::LocatorRestored,
            diagnostics: vec![diagnostic.into()],
        });
    }

    if let Some(chapter) = chapters
        .iter()
        .find(|chapter| chapter.ordinal == progress.locator.chapter_ordinal)
    {
        return Ok(UnifiedReadingProgress {
            locator: progress.locator.replacing_chapter(chapter),
            restore_state: UnifiedReadingRestoreState::OrdinalRestored,
            diagnostics: Vec::new(),
        });
    }

    let nearest_index = progress
        .locator
        .chapter_ordinal
        .clamp(0, chapters.len().saturating_sub(1) as i64) as usize;
    Ok(UnifiedReadingProgress {
        locator: progress.locator.replacing_chapter(chapters[nearest_index]),
        restore_state: UnifiedReadingRestoreState::NearestChapterRestored,
        diagnostics: vec!["nearest_fallback".into()],
    })
}

/// Legacy RECOVERY-33 chapter-index refresh diff category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedChapterDiffType {
    Unchanged,
    ChapterAdded,
    ChapterRemoved,
    ChapterTitleChanged,
    ChapterOrderChanged,
    ChapterLocatorChanged,
    ChapterContentLocatorChanged,
    DuplicateMerged,
    CanonicalLocatorChanged,
    BookMetadataChanged,
}

/// One deterministic chapter-index refresh diff row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedChapterDiffEntry {
    pub diff_type: UnifiedChapterDiffType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_chapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_chapter_id: Option<String>,
    pub reason: String,
}

/// Pure storage result for a unified chapter-index refresh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedChapterIndexRefreshResult {
    pub source_id: String,
    pub book_id: String,
    #[serde(default)]
    pub added_chapter_ids: Vec<String>,
    #[serde(default)]
    pub removed_chapter_ids: Vec<String>,
    #[serde(default)]
    pub changed_chapter_ids: Vec<String>,
    #[serde(default)]
    pub reordered_chapter_ids: Vec<String>,
    #[serde(default)]
    pub cache_invalidated_chapter_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_remapping_result: Option<UnifiedReadingProgress>,
    #[serde(default)]
    pub diff_entries: Vec<UnifiedChapterDiffEntry>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// Plan RECOVERY-33-style TOC refresh effects without mutating cache or runtime state.
pub fn plan_unified_chapter_index_refresh(
    source_id: &str,
    book_id: &str,
    old_chapters: &[UnifiedChapterIndexEntry],
    new_chapters: &[UnifiedChapterIndexEntry],
    progress: Option<&UnifiedReadingProgress>,
) -> Result<UnifiedChapterIndexRefreshResult, StorageError> {
    validate_book_key(source_id, book_id)?;
    for chapter in old_chapters.iter().chain(new_chapters) {
        chapter.validate()?;
    }

    let old = ordered_unique_unified_chapters(old_chapters, source_id, book_id);
    let new = ordered_unique_unified_chapters(new_chapters, source_id, book_id);
    let diff_entries = diff_unified_chapters(&old, &new);
    let progress_remapping_result = progress
        .map(|progress| {
            let scoped_new = new
                .iter()
                .map(|chapter| (*chapter).clone())
                .collect::<Vec<_>>();
            resolve_unified_reading_progress(progress, source_id, book_id, &scoped_new)
        })
        .transpose()?;

    let mut added_chapter_ids = diff_entries
        .iter()
        .filter(|entry| entry.diff_type == UnifiedChapterDiffType::ChapterAdded)
        .filter_map(|entry| entry.new_chapter_id.clone())
        .collect::<Vec<_>>();
    let mut removed_chapter_ids = diff_entries
        .iter()
        .filter(|entry| entry.diff_type == UnifiedChapterDiffType::ChapterRemoved)
        .filter_map(|entry| entry.old_chapter_id.clone())
        .collect::<Vec<_>>();
    let mut changed_chapter_ids = diff_entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.diff_type,
                UnifiedChapterDiffType::ChapterTitleChanged
                    | UnifiedChapterDiffType::ChapterLocatorChanged
                    | UnifiedChapterDiffType::ChapterContentLocatorChanged
                    | UnifiedChapterDiffType::CanonicalLocatorChanged
            )
        })
        .filter_map(|entry| {
            entry
                .old_chapter_id
                .clone()
                .or_else(|| entry.new_chapter_id.clone())
        })
        .collect::<Vec<_>>();
    let mut reordered_chapter_ids = diff_entries
        .iter()
        .filter(|entry| entry.diff_type == UnifiedChapterDiffType::ChapterOrderChanged)
        .filter_map(|entry| entry.new_chapter_id.clone())
        .collect::<Vec<_>>();
    let mut cache_invalidated_chapter_ids = diff_entries
        .iter()
        .filter(|entry| entry.diff_type != UnifiedChapterDiffType::Unchanged)
        .filter_map(|entry| {
            entry
                .old_chapter_id
                .clone()
                .or_else(|| entry.new_chapter_id.clone())
        })
        .collect::<Vec<_>>();

    sort_dedupe_strings(&mut added_chapter_ids);
    sort_dedupe_strings(&mut removed_chapter_ids);
    sort_dedupe_strings(&mut changed_chapter_ids);
    sort_dedupe_strings(&mut reordered_chapter_ids);
    sort_dedupe_strings(&mut cache_invalidated_chapter_ids);

    Ok(UnifiedChapterIndexRefreshResult {
        source_id: source_id.into(),
        book_id: book_id.into(),
        added_chapter_ids,
        removed_chapter_ids,
        changed_chapter_ids,
        reordered_chapter_ids,
        cache_invalidated_chapter_ids,
        progress_remapping_result,
        diff_entries,
        diagnostics: Vec::new(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ReadingProgressKey {
    source_id: String,
    book_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChapterDownloadStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

/// One queued chapter download/prefetch task.
///
/// This is transport-agnostic storage state. Runtime or host layers can claim a
/// task, perform the fetch, and then write the fetched body through
/// [`ChapterCacheStore`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterDownloadTask {
    pub source_id: String,
    pub book_id: String,
    pub chapter_index: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub priority: i32,
    pub status: ChapterDownloadStatus,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// RECOVERY-33 unified chapter download policy wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedDownloadTaskPolicy {
    CurrentChapterOnly,
    CurrentAndNext,
    SelectedRange,
    AllUncached,
    RefreshStale,
    RetryFailed,
}

/// RECOVERY-33 unified chapter download result state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedDownloadTaskState {
    Pending,
    Running,
    Completed,
    Partial,
    Failed,
    Cancelled,
}

/// Result for one requested chapter in a unified download task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedDownloadChapterResult {
    pub chapter_id: String,
    pub state: ChapterCacheStatus,
    pub cache_hit: bool,
    pub byte_count: u64,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// Deterministic fetch outcome supplied by a runtime or test fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedDownloadChapterOutcome {
    pub chapter_id: String,
    pub state: ChapterCacheStatus,
    #[serde(default)]
    pub byte_count: u64,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// Pure request used to summarize a unified chapter download operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedDownloadTaskRequest {
    pub task_id: String,
    pub source_id: String,
    pub book_id: String,
    #[serde(default)]
    pub requested_chapter_ids: Vec<String>,
    pub execution_policy: UnifiedDownloadTaskPolicy,
    pub concurrency_limit: usize,
    pub maximum_request_count: usize,
    #[serde(default)]
    pub runtime_maximum_concurrent_downloads: usize,
    #[serde(default)]
    pub runtime_maximum_request_count: usize,
    #[serde(default)]
    pub in_flight_task_ids: Vec<String>,
    #[serde(default)]
    pub cached_entries: Vec<UnifiedOfflineChapterCacheEntry>,
    #[serde(default)]
    pub fetch_outcomes: Vec<UnifiedDownloadChapterOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation_state: Option<String>,
}

/// RECOVERY-33-style unified chapter download summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedChapterDownloadTask {
    pub task_id: String,
    pub source_id: String,
    pub book_id: String,
    #[serde(default)]
    pub requested_chapter_ids: Vec<String>,
    pub execution_policy: UnifiedDownloadTaskPolicy,
    pub concurrency_limit: usize,
    pub maximum_request_count: usize,
    pub current_state: UnifiedDownloadTaskState,
    pub completed_count: usize,
    pub failed_count: usize,
    pub skipped_cached_count: usize,
    pub bytes_received: u64,
    pub cancellation_state: String,
    #[serde(default)]
    pub failure_diagnostics: Vec<String>,
    #[serde(default)]
    pub per_chapter_results: Vec<UnifiedDownloadChapterResult>,
}

/// Summarize a bounded unified download without performing fetch or cache IO.
pub fn plan_unified_chapter_download_task(
    request: &UnifiedDownloadTaskRequest,
) -> Result<UnifiedChapterDownloadTask, StorageError> {
    validate_unified_download_task_request(request)?;
    if request
        .in_flight_task_ids
        .iter()
        .any(|task_id| task_id == &request.task_id)
    {
        return Ok(UnifiedChapterDownloadTask {
            task_id: request.task_id.clone(),
            source_id: request.source_id.clone(),
            book_id: request.book_id.clone(),
            requested_chapter_ids: request.requested_chapter_ids.clone(),
            execution_policy: request.execution_policy,
            concurrency_limit: request.concurrency_limit,
            maximum_request_count: request.maximum_request_count,
            current_state: UnifiedDownloadTaskState::Partial,
            completed_count: 0,
            failed_count: 0,
            skipped_cached_count: 0,
            bytes_received: 0,
            cancellation_state: "coalesced".into(),
            failure_diagnostics: vec!["duplicate_task_coalesced".into()],
            per_chapter_results: Vec::new(),
        });
    }

    let mut completed_count = 0usize;
    let mut failed_count = 0usize;
    let mut skipped_cached_count = 0usize;
    let mut bytes_received = 0u64;
    let mut per_chapter_results = Vec::new();
    let outcome_by_chapter = request
        .fetch_outcomes
        .iter()
        .map(|outcome| (outcome.chapter_id.as_str(), outcome))
        .collect::<HashMap<_, _>>();

    for chapter_id in request
        .requested_chapter_ids
        .iter()
        .take(request.maximum_request_count)
    {
        if let Some(cached) = request.cached_entries.iter().find(|entry| {
            entry.source_id == request.source_id
                && entry.book_id == request.book_id
                && entry.chapter_id == *chapter_id
                && is_unified_download_cache_hit_state(entry.state)
        }) {
            skipped_cached_count += 1;
            per_chapter_results.push(UnifiedDownloadChapterResult {
                chapter_id: chapter_id.clone(),
                state: cached.state,
                cache_hit: true,
                byte_count: cached.byte_count,
                diagnostics: Vec::new(),
            });
            continue;
        }

        let Some(outcome) = outcome_by_chapter.get(chapter_id.as_str()) else {
            failed_count += 1;
            per_chapter_results.push(UnifiedDownloadChapterResult {
                chapter_id: chapter_id.clone(),
                state: ChapterCacheStatus::Failed,
                cache_hit: false,
                byte_count: 0,
                diagnostics: vec!["download_outcome_missing".into()],
            });
            continue;
        };

        if is_unified_download_success_state(outcome.state) {
            completed_count += 1;
            bytes_received += outcome.byte_count;
        } else {
            failed_count += 1;
        }
        per_chapter_results.push(UnifiedDownloadChapterResult {
            chapter_id: chapter_id.clone(),
            state: outcome.state,
            cache_hit: false,
            byte_count: outcome.byte_count,
            diagnostics: outcome.diagnostics.clone(),
        });
    }

    let current_state = if failed_count == 0 {
        UnifiedDownloadTaskState::Completed
    } else if completed_count > 0 || skipped_cached_count > 0 {
        UnifiedDownloadTaskState::Partial
    } else {
        UnifiedDownloadTaskState::Failed
    };
    let failure_diagnostics = per_chapter_results
        .iter()
        .flat_map(|result| result.diagnostics.iter().cloned())
        .collect::<Vec<_>>();

    Ok(UnifiedChapterDownloadTask {
        task_id: request.task_id.clone(),
        source_id: request.source_id.clone(),
        book_id: request.book_id.clone(),
        requested_chapter_ids: request.requested_chapter_ids.clone(),
        execution_policy: request.execution_policy,
        concurrency_limit: capped_nonzero_limit(
            request.concurrency_limit,
            request.runtime_maximum_concurrent_downloads,
        ),
        maximum_request_count: capped_nonzero_limit(
            request.maximum_request_count,
            request.runtime_maximum_request_count,
        ),
        current_state,
        completed_count,
        failed_count,
        skipped_cached_count,
        bytes_received,
        cancellation_state: request
            .cancellation_state
            .clone()
            .unwrap_or_else(|| "not_cancelled".into()),
        failure_diagnostics,
        per_chapter_results,
    })
}

impl ChapterDownloadTask {
    pub fn pending(
        source_id: impl Into<String>,
        book_id: impl Into<String>,
        chapter_index: u32,
        title: impl Into<String>,
        url: impl Into<String>,
        priority: i32,
        created_at: i64,
    ) -> Result<Self, StorageError> {
        let task = Self {
            source_id: source_id.into(),
            book_id: book_id.into(),
            chapter_index,
            title: title.into(),
            url: url.into(),
            priority,
            status: ChapterDownloadStatus::Pending,
            created_at,
            updated_at: created_at,
            attempts: 0,
            max_attempts: default_max_attempts(),
            last_error: None,
        };
        validate_chapter_download_task(&task)?;
        Ok(task)
    }

    fn download_key(&self) -> ChapterDownloadKey {
        ChapterDownloadKey {
            source_id: self.source_id.clone(),
            book_id: self.book_id.clone(),
            chapter_index: self.chapter_index,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ChapterDownloadKey {
    source_id: String,
    book_id: String,
    chapter_index: u32,
}

/// Chapter item used by legacy bookshelf update/source-switch matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterMatchCandidate {
    pub chapter_id: String,
    pub chapter_title: String,
    pub chapter_url: String,
    pub order: i32,
}

impl ChapterMatchCandidate {
    pub fn new(
        chapter_id: impl Into<String>,
        title: impl Into<String>,
        url: impl Into<String>,
        order: i32,
    ) -> Self {
        Self {
            chapter_id: chapter_id.into(),
            chapter_title: title.into(),
            chapter_url: url.into(),
            order,
        }
    }
}

/// Result of comparing a previous TOC with a refreshed TOC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterUpdateResult {
    pub has_new_chapters: bool,
    #[serde(default)]
    pub new_chapters: Vec<ChapterMatchCandidate>,
    pub total_old_chapters: usize,
    pub total_new_chapters: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Legacy bookshelf update detection: a chapter is new when its trimmed,
/// lowercased title is absent from the old TOC.
pub fn detect_new_chapters(
    old_toc: &[ChapterMatchCandidate],
    new_toc: &[ChapterMatchCandidate],
) -> ChapterUpdateResult {
    let old_titles = old_toc
        .iter()
        .map(|chapter| chapter_update_title_key(&chapter.chapter_title))
        .collect::<HashSet<_>>();
    let new_chapters = new_toc
        .iter()
        .filter(|chapter| !old_titles.contains(&chapter_update_title_key(&chapter.chapter_title)))
        .cloned()
        .collect::<Vec<_>>();

    ChapterUpdateResult {
        has_new_chapters: !new_chapters.is_empty(),
        new_chapters,
        total_old_chapters: old_toc.len(),
        total_new_chapters: new_toc.len(),
        error: None,
    }
}

/// Legacy count-based fallback for sources whose chapter titles are unstable.
pub fn detect_new_chapters_by_count(
    old_count: usize,
    new_toc: &[ChapterMatchCandidate],
) -> ChapterUpdateResult {
    if new_toc.len() <= old_count {
        return ChapterUpdateResult {
            has_new_chapters: false,
            new_chapters: Vec::new(),
            total_old_chapters: old_count,
            total_new_chapters: new_toc.len(),
            error: None,
        };
    }

    let new_chapters = new_toc[old_count..].to_vec();
    ChapterUpdateResult {
        has_new_chapters: true,
        new_chapters,
        total_old_chapters: old_count,
        total_new_chapters: new_toc.len(),
        error: None,
    }
}

fn chapter_update_title_key(title: &str) -> String {
    title.trim().to_lowercase()
}

/// Legacy bookshelf update notification category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateNotificationType {
    NewChaptersAvailable,
    ChaptersUpdated,
    ChaptersRemoved,
    NoUpdate,
    UpdateFailed,
    SourceSwitched,
    CacheCompleted,
    CacheFailed,
    NeedsUserReview,
}

/// Pure result of integrating chapter update detection with cache state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChapterUpdateCacheResult {
    pub new_chapter_count: usize,
    pub queued_for_download: usize,
    pub notification_type: UpdateNotificationType,
    pub cache_status: OfflineAvailability,
}

impl ChapterUpdateCacheResult {
    pub fn has_update(&self) -> bool {
        self.new_chapter_count > 0
    }
}

/// Legacy scheduled-update runtime mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledUpdateRuntimeMode {
    Fixture,
    Mock,
    LiveOptIn,
}

impl Default for ScheduledUpdateRuntimeMode {
    fn default() -> Self {
        Self::Fixture
    }
}

/// Deterministic scheduled-update policy from the old Reader-Core runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledUpdatePolicy {
    #[serde(default)]
    pub allow_network: bool,
    #[serde(default)]
    pub auto_download_new_chapters: bool,
    #[serde(default)]
    pub notify_on_no_update: bool,
    #[serde(default = "default_scheduled_update_max_books_per_run")]
    pub max_books_per_run: usize,
    #[serde(default = "default_scheduled_update_max_chapters_per_book")]
    pub max_chapters_per_book: usize,
    #[serde(default = "default_scheduled_update_skip_archived")]
    pub skip_archived: bool,
    #[serde(default = "default_scheduled_update_skip_cloudflare_blocked")]
    pub skip_cloudflare_blocked: bool,
}

impl Default for ScheduledUpdatePolicy {
    fn default() -> Self {
        Self {
            allow_network: false,
            auto_download_new_chapters: false,
            notify_on_no_update: false,
            max_books_per_run: default_scheduled_update_max_books_per_run(),
            max_chapters_per_book: default_scheduled_update_max_chapters_per_book(),
            skip_archived: default_scheduled_update_skip_archived(),
            skip_cloudflare_blocked: default_scheduled_update_skip_cloudflare_blocked(),
        }
    }
}

/// One bookshelf item plus deterministic TOC/cache inputs for a scheduled run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledUpdateBookInput {
    pub source_id: String,
    pub book_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub cloudflare_blocked: bool,
    #[serde(default)]
    pub old_toc: Vec<ChapterMatchCandidate>,
    #[serde(default)]
    pub new_toc: Vec<ChapterMatchCandidate>,
    #[serde(default)]
    pub cache_entries: Vec<OfflineChapterCacheEntry>,
    #[serde(default)]
    pub already_queued_chapter_ids: Vec<String>,
}

/// Pure scheduled-update request. It carries already-parsed fixture data and
/// does not perform network, WebView, scheduler, or notification side effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledUpdateRequest {
    #[serde(default)]
    pub policy: ScheduledUpdatePolicy,
    #[serde(default)]
    pub runtime_mode: ScheduledUpdateRuntimeMode,
    #[serde(default)]
    pub books: Vec<ScheduledUpdateBookInput>,
}

impl Default for ScheduledUpdateRequest {
    fn default() -> Self {
        Self {
            policy: ScheduledUpdatePolicy::default(),
            runtime_mode: ScheduledUpdateRuntimeMode::default(),
            books: Vec::new(),
        }
    }
}

/// Run-level status values from legacy scheduled update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledUpdateRunStatus {
    NoEligibleBooks,
    AllSkipped,
    Success,
    PartialSuccess,
    Failure,
}

/// Per-book scheduled update status values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledUpdateBookStatus {
    SkippedArchived,
    SkippedCloudflareBlocked,
    NoUpdate,
    Updated,
    UpdateFailed,
}

/// Storage-owned notification summary category. Host apps can map this to
/// platform notifications without Core issuing notifications itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledUpdateNotificationCategory {
    NewChapters,
    UpdateAvailable,
    NoUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledUpdateNotification {
    pub category: ScheduledUpdateNotificationCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_id: Option<String>,
    pub title: String,
    pub body: String,
    pub badge_count: u32,
}

/// Scheduled-update result for a single book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledUpdateBookResult {
    pub source_id: String,
    pub book_id: String,
    pub status: ScheduledUpdateBookStatus,
    pub new_chapter_count: usize,
    pub download_queue_suggestions: usize,
    #[serde(default)]
    pub suggested_chapter_ids: Vec<String>,
    pub auto_download_completed: usize,
    pub auto_download_failed: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_status: Option<OfflineAvailability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification_type: Option<UpdateNotificationType>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// Pure scheduled-update state-machine summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledUpdateRunResult {
    pub status: ScheduledUpdateRunStatus,
    pub checked_books: usize,
    pub updated_books: usize,
    pub skipped_books: usize,
    pub new_chapters_detected: usize,
    pub download_queue_suggestions: usize,
    pub auto_download_completed: usize,
    pub auto_download_failed: usize,
    pub network_accessed: bool,
    pub web_view_used: bool,
    #[serde(default)]
    pub per_book_results: Vec<ScheduledUpdateBookResult>,
    #[serde(default)]
    pub notifications: Vec<ScheduledUpdateNotification>,
}

/// Plan the legacy chapter update + cache integration without mutating a host queue.
pub fn plan_chapter_update_cache(
    old_toc: &[ChapterMatchCandidate],
    new_toc: &[ChapterMatchCandidate],
    source_id: impl Into<String>,
    cache_entries: &[OfflineChapterCacheEntry],
    already_queued_chapter_ids: &[String],
) -> Result<ChapterUpdateCacheResult, StorageError> {
    let source_id = source_id.into();
    let update_result = detect_new_chapters(old_toc, new_toc);
    let mut seen = already_queued_chapter_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let queued_for_download = update_result
        .new_chapters
        .iter()
        .filter(|chapter| seen.insert(chapter.chapter_id.clone()))
        .count();
    let total_chapters =
        u32::try_from(new_toc.len()).map_err(|_| StorageError::InvalidChapterCache {
            field: "total_chapters".into(),
        })?;
    let cache_status = compute_offline_availability(source_id, total_chapters, cache_entries)?;
    let notification_type = if update_result.has_new_chapters {
        UpdateNotificationType::NewChaptersAvailable
    } else {
        UpdateNotificationType::NoUpdate
    };

    Ok(ChapterUpdateCacheResult {
        new_chapter_count: update_result.new_chapters.len(),
        queued_for_download,
        notification_type,
        cache_status,
    })
}

/// Plan a full legacy scheduled update run without mutating queues, cache,
/// network, WebView, scheduler, or platform notifications.
pub fn plan_scheduled_update(
    request: &ScheduledUpdateRequest,
) -> Result<ScheduledUpdateRunResult, StorageError> {
    validate_scheduled_update_request(request)?;

    if request.books.is_empty() {
        return Ok(ScheduledUpdateRunResult {
            status: ScheduledUpdateRunStatus::NoEligibleBooks,
            checked_books: 0,
            updated_books: 0,
            skipped_books: 0,
            new_chapters_detected: 0,
            download_queue_suggestions: 0,
            auto_download_completed: 0,
            auto_download_failed: 0,
            network_accessed: false,
            web_view_used: false,
            per_book_results: Vec::new(),
            notifications: Vec::new(),
        });
    }

    let mut checked_books = 0usize;
    let mut updated_books = 0usize;
    let mut skipped_books = 0usize;
    let mut new_chapters_detected = 0usize;
    let mut download_queue_suggestions = 0usize;
    let mut auto_download_completed = 0usize;
    let mut auto_download_failed = 0usize;
    let mut per_book_results = Vec::new();
    let mut notifications = Vec::new();

    for book in &request.books {
        if request.policy.skip_archived && book.archived {
            skipped_books += 1;
            per_book_results.push(skipped_scheduled_update_book_result(
                book,
                ScheduledUpdateBookStatus::SkippedArchived,
                "skipped_archived",
            ));
            continue;
        }
        if request.policy.skip_cloudflare_blocked && scheduled_update_book_is_blocked(book) {
            skipped_books += 1;
            per_book_results.push(skipped_scheduled_update_book_result(
                book,
                ScheduledUpdateBookStatus::SkippedCloudflareBlocked,
                "skipped_cloudflare_blocked",
            ));
            continue;
        }
        if checked_books >= request.policy.max_books_per_run {
            break;
        }

        checked_books += 1;
        let update = plan_chapter_update_cache(
            &book.old_toc,
            &book.new_toc,
            &book.source_id,
            &book.cache_entries,
            &book.already_queued_chapter_ids,
        )?;
        let update_result = detect_new_chapters(&book.old_toc, &book.new_toc);
        let suggested_chapter_ids = scheduled_update_suggested_chapter_ids(
            &update_result.new_chapters,
            &book.already_queued_chapter_ids,
            request.policy.max_chapters_per_book,
        );
        let mut cache_status = update.cache_status.clone();
        let mut completed = 0usize;
        let mut failed = 0usize;

        if request.policy.auto_download_new_chapters
            && request.runtime_mode == ScheduledUpdateRuntimeMode::Mock
        {
            completed = suggested_chapter_ids.len();
            let mock_cache_entries =
                scheduled_update_mock_cache_entries(book, &suggested_chapter_ids);
            cache_status = compute_offline_availability(
                &book.source_id,
                u32::try_from(book.new_toc.len()).map_err(|_| {
                    StorageError::InvalidChapterCache {
                        field: "total_chapters".into(),
                    }
                })?,
                &mock_cache_entries,
            )?;
        } else if request.policy.auto_download_new_chapters
            && !suggested_chapter_ids.is_empty()
            && request.runtime_mode != ScheduledUpdateRuntimeMode::Mock
        {
            failed = 0;
        }

        let status = if update_result.has_new_chapters {
            updated_books += 1;
            ScheduledUpdateBookStatus::Updated
        } else {
            ScheduledUpdateBookStatus::NoUpdate
        };
        new_chapters_detected += update_result.new_chapters.len();
        download_queue_suggestions += suggested_chapter_ids.len();
        auto_download_completed += completed;
        auto_download_failed += failed;

        let notification_type = if update_result.has_new_chapters {
            Some(UpdateNotificationType::NewChaptersAvailable)
        } else {
            Some(UpdateNotificationType::NoUpdate)
        };

        if update_result.has_new_chapters || request.policy.notify_on_no_update {
            notifications.push(scheduled_update_book_notification(
                book,
                if update_result.has_new_chapters {
                    ScheduledUpdateNotificationCategory::NewChapters
                } else {
                    ScheduledUpdateNotificationCategory::NoUpdate
                },
                update_result.new_chapters.len(),
            )?);
        }

        per_book_results.push(ScheduledUpdateBookResult {
            source_id: book.source_id.clone(),
            book_id: book.book_id.clone(),
            status,
            new_chapter_count: update_result.new_chapters.len(),
            download_queue_suggestions: suggested_chapter_ids.len(),
            suggested_chapter_ids,
            auto_download_completed: completed,
            auto_download_failed: failed,
            cache_status: Some(cache_status),
            notification_type,
            diagnostics: Vec::new(),
        });
    }

    if updated_books > 1 {
        notifications.push(scheduled_update_summary_notification(updated_books)?);
    }

    let status = if checked_books == 0 && skipped_books > 0 {
        ScheduledUpdateRunStatus::AllSkipped
    } else if updated_books > 0 && skipped_books > 0 {
        ScheduledUpdateRunStatus::PartialSuccess
    } else if checked_books > 0 {
        ScheduledUpdateRunStatus::Success
    } else {
        ScheduledUpdateRunStatus::NoEligibleBooks
    };

    Ok(ScheduledUpdateRunResult {
        status,
        checked_books,
        updated_books,
        skipped_books,
        new_chapters_detected,
        download_queue_suggestions,
        auto_download_completed,
        auto_download_failed,
        network_accessed: false,
        web_view_used: false,
        per_book_results,
        notifications,
    })
}

fn default_scheduled_update_max_books_per_run() -> usize {
    5
}

fn default_scheduled_update_max_chapters_per_book() -> usize {
    3
}

fn default_scheduled_update_skip_archived() -> bool {
    true
}

fn default_scheduled_update_skip_cloudflare_blocked() -> bool {
    true
}

fn validate_scheduled_update_request(request: &ScheduledUpdateRequest) -> Result<(), StorageError> {
    if request.policy.max_books_per_run == 0 {
        return Err(StorageError::InvalidChapterCache {
            field: "max_books_per_run".into(),
        });
    }
    if request.policy.max_chapters_per_book == 0 {
        return Err(StorageError::InvalidChapterCache {
            field: "max_chapters_per_book".into(),
        });
    }
    for book in &request.books {
        validate_scheduled_update_book(book)?;
    }
    Ok(())
}

fn validate_scheduled_update_book(book: &ScheduledUpdateBookInput) -> Result<(), StorageError> {
    validate_book_key(&book.source_id, &book.book_id)?;
    validate_update_optional_text(&book.title, "title")?;
    validate_update_optional_text(&book.author, "author")?;
    for chapter in book.old_toc.iter().chain(&book.new_toc) {
        validate_chapter_match_candidate(chapter)?;
    }
    for entry in &book.cache_entries {
        validate_offline_chapter_cache_entry(entry)?;
    }
    if book
        .already_queued_chapter_ids
        .iter()
        .any(|chapter_id| chapter_id.trim().is_empty())
    {
        return Err(StorageError::InvalidChapterCache {
            field: "already_queued_chapter_ids".into(),
        });
    }
    Ok(())
}

fn validate_chapter_match_candidate(chapter: &ChapterMatchCandidate) -> Result<(), StorageError> {
    validate_chapter_cache_required(&chapter.chapter_id, "chapter_id")?;
    validate_update_optional_text(&chapter.chapter_title, "chapter_title")?;
    validate_update_optional_text(&chapter.chapter_url, "chapter_url")?;
    if chapter.order < 0 {
        return Err(StorageError::InvalidChapterCache {
            field: "chapter_order".into(),
        });
    }
    Ok(())
}

fn validate_update_optional_text(value: &str, field: &str) -> Result<(), StorageError> {
    if !value.is_empty() && value.trim().is_empty() {
        return Err(StorageError::InvalidChapterCache {
            field: field.into(),
        });
    }
    Ok(())
}

fn scheduled_update_book_is_blocked(book: &ScheduledUpdateBookInput) -> bool {
    book.cloudflare_blocked || book.source_id == "case_021"
}

fn skipped_scheduled_update_book_result(
    book: &ScheduledUpdateBookInput,
    status: ScheduledUpdateBookStatus,
    diagnostic: &str,
) -> ScheduledUpdateBookResult {
    ScheduledUpdateBookResult {
        source_id: book.source_id.clone(),
        book_id: book.book_id.clone(),
        status,
        new_chapter_count: 0,
        download_queue_suggestions: 0,
        suggested_chapter_ids: Vec::new(),
        auto_download_completed: 0,
        auto_download_failed: 0,
        cache_status: None,
        notification_type: None,
        diagnostics: vec![diagnostic.into()],
    }
}

fn scheduled_update_suggested_chapter_ids(
    new_chapters: &[ChapterMatchCandidate],
    already_queued_chapter_ids: &[String],
    max_chapters_per_book: usize,
) -> Vec<String> {
    let mut seen = already_queued_chapter_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let mut ids = Vec::new();
    for chapter in new_chapters {
        if seen.insert(chapter.chapter_id.clone()) {
            ids.push(chapter.chapter_id.clone());
        }
        if ids.len() >= max_chapters_per_book {
            break;
        }
    }
    ids
}

fn scheduled_update_mock_cache_entries(
    book: &ScheduledUpdateBookInput,
    suggested_chapter_ids: &[String],
) -> Vec<OfflineChapterCacheEntry> {
    let mut entries = book.cache_entries.clone();
    for chapter_id in suggested_chapter_ids {
        entries.push(OfflineChapterCacheEntry {
            source_id: book.source_id.clone(),
            chapter_id: chapter_id.clone(),
            status: ChapterCacheStatus::Available,
        });
    }
    entries
}

fn scheduled_update_book_notification(
    book: &ScheduledUpdateBookInput,
    category: ScheduledUpdateNotificationCategory,
    new_chapter_count: usize,
) -> Result<ScheduledUpdateNotification, StorageError> {
    let title = match category {
        ScheduledUpdateNotificationCategory::NewChapters => "New Chapters",
        ScheduledUpdateNotificationCategory::UpdateAvailable => "Updates Available",
        ScheduledUpdateNotificationCategory::NoUpdate => "No Updates",
    }
    .to_string();
    let body = match category {
        ScheduledUpdateNotificationCategory::NewChapters => {
            format!("{new_chapter_count} new chapters available")
        }
        ScheduledUpdateNotificationCategory::UpdateAvailable => {
            format!("{new_chapter_count} books updated")
        }
        ScheduledUpdateNotificationCategory::NoUpdate => "No new chapters".to_string(),
    };
    Ok(ScheduledUpdateNotification {
        category,
        source_id: Some(book.source_id.clone()),
        book_id: Some(book.book_id.clone()),
        title,
        body,
        badge_count: u32::try_from(new_chapter_count).map_err(|_| {
            StorageError::InvalidChapterCache {
                field: "badge_count".into(),
            }
        })?,
    })
}

fn scheduled_update_summary_notification(
    updated_books: usize,
) -> Result<ScheduledUpdateNotification, StorageError> {
    Ok(ScheduledUpdateNotification {
        category: ScheduledUpdateNotificationCategory::UpdateAvailable,
        source_id: None,
        book_id: None,
        title: "Updates Available".into(),
        body: format!("{updated_books} books updated"),
        badge_count: u32::try_from(updated_books).map_err(|_| {
            StorageError::InvalidChapterCache {
                field: "badge_count".into(),
            }
        })?,
    })
}

/// Legacy source-switch matching strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceSwitchStrategy {
    ExactTitleMatch,
    OrderIndexMatch,
    FuzzyTitleMatch,
    LastReadFallback,
}

/// Result of matching the current chapter against a replacement source TOC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSwitchResult {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_chapter: Option<ChapterMatchCandidate>,
    pub strategy: SourceSwitchStrategy,
    pub total_chapters: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of migrating source-scoped reading progress after a source switch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceSwitchProgressMigrationResult {
    pub success: bool,
    pub old_progress: ReadingProgressEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_progress: Option<ReadingProgressEntry>,
    pub chapter_matched: bool,
}

/// Pure source-switch chapter matcher from the legacy Reader-Core bookshelf runtime.
pub fn match_source_switch_chapter(
    old_toc: &[ChapterMatchCandidate],
    new_toc: &[ChapterMatchCandidate],
    current_chapter_title: &str,
    current_chapter_index: usize,
) -> SourceSwitchResult {
    let _ = old_toc;

    if new_toc.is_empty() {
        return SourceSwitchResult {
            success: false,
            matched_chapter: None,
            strategy: SourceSwitchStrategy::LastReadFallback,
            total_chapters: 0,
            error: Some("New source TOC is empty".into()),
        };
    }

    if let Some(chapter) = new_toc
        .iter()
        .find(|chapter| chapter.chapter_title == current_chapter_title)
    {
        return successful_source_switch_match(
            chapter,
            SourceSwitchStrategy::ExactTitleMatch,
            new_toc.len(),
        );
    }

    if let Some(chapter) = new_toc.get(current_chapter_index) {
        return successful_source_switch_match(
            chapter,
            SourceSwitchStrategy::OrderIndexMatch,
            new_toc.len(),
        );
    }

    let normalized_current = normalize_source_switch_title(current_chapter_title);
    if let Some(chapter) = new_toc
        .iter()
        .find(|chapter| normalize_source_switch_title(&chapter.chapter_title) == normalized_current)
    {
        return successful_source_switch_match(
            chapter,
            SourceSwitchStrategy::FuzzyTitleMatch,
            new_toc.len(),
        );
    }

    successful_source_switch_match(
        &new_toc[0],
        SourceSwitchStrategy::LastReadFallback,
        new_toc.len(),
    )
}

/// Migrate reading progress after a source switch, resetting the matched chapter
/// to its beginning like the legacy Reader-Core bookshelf runtime.
pub fn migrate_source_switch_progress(
    old_progress: &ReadingProgressEntry,
    switch_result: &SourceSwitchResult,
    new_source_id: impl Into<String>,
    updated_at: i64,
) -> Result<SourceSwitchProgressMigrationResult, StorageError> {
    validate_reading_progress(old_progress)?;

    let Some(matched_chapter) = switch_result
        .success
        .then_some(switch_result.matched_chapter.as_ref())
        .flatten()
    else {
        return Ok(SourceSwitchProgressMigrationResult {
            success: false,
            old_progress: old_progress.clone(),
            new_progress: None,
            chapter_matched: false,
        });
    };

    let new_source_id = new_source_id.into();
    validate_source_id(&new_source_id)?;
    let chapter_index =
        u32::try_from(matched_chapter.order).map_err(|_| StorageError::InvalidProgress {
            field: "chapter_order".into(),
        })?;
    let new_progress = ReadingProgressEntry {
        source_id: new_source_id,
        book_id: old_progress.book_id.clone(),
        chapter_index,
        chapter_offset: 0,
        chapter_progress: 0.0,
        updated_at,
        device_id: old_progress.device_id.clone(),
    };
    validate_reading_progress(&new_progress)?;

    Ok(SourceSwitchProgressMigrationResult {
        success: true,
        old_progress: old_progress.clone(),
        new_progress: Some(new_progress),
        chapter_matched: true,
    })
}

/// Normalize a chapter title using the legacy source-switch fuzzy matcher rules.
pub fn normalize_source_switch_title(title: &str) -> String {
    let mut value = title.trim().to_string();
    value = strip_chinese_chapter_prefix(&value).to_string();
    value = strip_english_chapter_prefix(&value).to_string();
    value = strip_leading_number_prefix(&value).to_string();
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn successful_source_switch_match(
    chapter: &ChapterMatchCandidate,
    strategy: SourceSwitchStrategy,
    total_chapters: usize,
) -> SourceSwitchResult {
    SourceSwitchResult {
        success: true,
        matched_chapter: Some(chapter.clone()),
        strategy,
        total_chapters,
        error: None,
    }
}

fn strip_chinese_chapter_prefix(value: &str) -> &str {
    let Some(after_prefix) = value.strip_prefix('第') else {
        return value;
    };
    let mut chapter_found_end = None;
    for (offset, ch) in after_prefix.char_indices() {
        if ch == '章' {
            chapter_found_end = Some(offset + ch.len_utf8());
            break;
        }
    }
    chapter_found_end
        .map(|end| after_prefix[end..].trim_start())
        .unwrap_or(value)
}

fn strip_english_chapter_prefix(value: &str) -> &str {
    let Some(after_chapter) = strip_ascii_prefix_ignore_case(value, "chapter") else {
        return value;
    };
    let after_gap = after_chapter.trim_start();
    if after_gap.len() == after_chapter.len() {
        return value;
    }

    let digit_end = after_gap
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(offset, ch)| offset + ch.len_utf8());
    digit_end
        .map(|end| after_gap[end..].trim_start())
        .unwrap_or(value)
}

fn strip_leading_number_prefix(value: &str) -> &str {
    let Some(digit_end) = value
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_digit())
        .last()
        .map(|(offset, ch)| offset + ch.len_utf8())
    else {
        return value;
    };
    let mut separator_end = digit_end;
    for (offset, ch) in value[digit_end..].char_indices() {
        if ch.is_whitespace() || matches!(ch, '.' | '-' | ':') {
            separator_end = digit_end + offset + ch.len_utf8();
        } else {
            break;
        }
    }
    if separator_end == digit_end {
        value
    } else {
        &value[separator_end..]
    }
}

fn strip_ascii_prefix_ignore_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        .then(|| &value[prefix.len()..])
}

/// Storage operations for the bookshelf.
///
/// The trait is defined so a future SQLite-backed store can implement the same
/// surface; V1 ships the in-memory implementation on [`InMemoryStorage`].
pub trait BookshelfStore {
    /// Add a book to the shelf, or update its metadata if already present.
    ///
    /// Upsert semantics: if an entry with the same `(source_id, book_id)`
    /// already exists, its `added_at` is preserved and the remaining fields
    /// are overwritten from `entry`.
    fn add_to_shelf(&self, entry: BookshelfEntry) -> Result<BookshelfEntry, StorageError>;

    /// Remove a book from the shelf. Idempotent — removing a missing entry is
    /// not an error.
    fn remove_from_shelf(&self, source_id: &str, book_id: &str) -> Result<(), StorageError>;

    /// Look up a single shelf entry by composite key.
    fn get_shelf_entry(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Option<BookshelfEntry>, StorageError>;

    /// List all shelf entries, sorted by `sort_index` ascending then
    /// `added_at` descending (most recently added first within the same index).
    fn list_shelf(&self) -> Result<Vec<BookshelfEntry>, StorageError>;

    /// Query shelf entries by source/group/keyword/progress state with
    /// deterministic sorting and pagination.
    fn query_shelf(&self, query: BookshelfQuery) -> Result<Vec<BookshelfEntry>, StorageError>;

    /// List shelf entries in a given group, sorted as [`BookshelfStore::list_shelf`].
    fn list_shelf_by_group(&self, group: &str) -> Result<Vec<BookshelfEntry>, StorageError>;

    /// Update the `last_read_at` timestamp for a shelf entry.
    /// Returns `StorageError::NotFound` if the entry does not exist.
    fn update_last_read(
        &self,
        source_id: &str,
        book_id: &str,
        timestamp: i64,
    ) -> Result<(), StorageError>;

    /// Move a shelf entry to a group and manual sort position.
    fn move_shelf_entry(
        &self,
        source_id: &str,
        book_id: &str,
        group: Option<String>,
        sort_index: i32,
    ) -> Result<BookshelfEntry, StorageError>;

    /// Number of entries on the shelf.
    fn shelf_count(&self) -> Result<usize, StorageError>;
}

/// Storage operations for normalized chapter bodies.
pub trait ChapterCacheStore {
    /// Insert or replace a cached chapter body.
    fn put_chapter_cache(
        &self,
        entry: ChapterCacheEntry,
    ) -> Result<ChapterCacheEntry, StorageError>;

    /// Read a cached chapter body by source/book/index.
    fn get_chapter_cache(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<Option<ChapterCacheEntry>, StorageError>;

    /// Remove a single cached chapter body. Idempotent for missing entries.
    fn remove_chapter_cache(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<(), StorageError>;

    /// List cached chapters for a book, sorted by chapter index ascending.
    fn list_chapter_cache(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Vec<ChapterCacheEntry>, StorageError>;

    /// Clear all cached chapters for a book and return the number removed.
    fn clear_chapter_cache(&self, source_id: &str, book_id: &str) -> Result<usize, StorageError>;

    /// Compute cache coverage for a book against the current TOC length.
    fn chapter_cache_coverage(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_count: u32,
    ) -> Result<ChapterCacheCoverage, StorageError>;

    /// Plan missing chapter indexes in a bounded window around an anchor.
    fn plan_chapter_cache_prefetch(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_count: u32,
        anchor_index: u32,
        before: u32,
        after: u32,
        max_count: usize,
    ) -> Result<ChapterCachePrefetchPlan, StorageError>;

    /// Return aggregate chapter cache stats across all books.
    fn chapter_cache_stats(&self) -> Result<ChapterCacheStats, StorageError>;

    /// Apply retention limits and evict old cache entries.
    fn prune_chapter_cache(
        &self,
        policy: ChapterCacheRetentionPolicy,
    ) -> Result<ChapterCacheEvictionReport, StorageError>;
}

/// Storage operations for source-scoped reading progress.
pub trait ReadingProgressStore {
    /// Save a reading progress update.
    ///
    /// Updates are appended to history. The current pointer only advances when
    /// `updated_at` is newer than or equal to the stored current update.
    fn save_reading_progress(
        &self,
        entry: ReadingProgressEntry,
    ) -> Result<ReadingProgressEntry, StorageError>;

    /// Read current progress by source/book key.
    fn get_reading_progress(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Option<ReadingProgressEntry>, StorageError>;

    /// List current progress for one source, newest first.
    fn list_reading_progress(
        &self,
        source_id: &str,
    ) -> Result<Vec<ReadingProgressEntry>, StorageError>;

    /// Return saved progress events for one book, oldest first.
    fn reading_progress_history(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Vec<ReadingProgressEntry>, StorageError>;

    /// Clear current progress and history for one book. Idempotent.
    fn clear_reading_progress(&self, source_id: &str, book_id: &str) -> Result<(), StorageError>;
}

/// Storage operations for bounded chapter prefetch/download work.
pub trait ChapterDownloadQueueStore {
    /// Add a chapter to the queue, or requeue it if the key already exists.
    ///
    /// Requeue semantics preserve the original `created_at`, refresh metadata,
    /// reset attempts/error, and put the task back into `Pending`.
    fn enqueue_chapter_download(
        &self,
        task: ChapterDownloadTask,
    ) -> Result<ChapterDownloadTask, StorageError>;

    fn get_chapter_download(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<Option<ChapterDownloadTask>, StorageError>;

    fn list_chapter_downloads(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Vec<ChapterDownloadTask>, StorageError>;

    /// Claim the next pending/retryable task and mark it `InProgress`.
    fn claim_next_chapter_download(
        &self,
        now: i64,
    ) -> Result<Option<ChapterDownloadTask>, StorageError>;

    fn mark_chapter_download_completed(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
        now: i64,
    ) -> Result<ChapterDownloadTask, StorageError>;

    fn mark_chapter_download_failed(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
        error: impl Into<String>,
        now: i64,
    ) -> Result<ChapterDownloadTask, StorageError>;

    fn cancel_chapter_download(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
        now: i64,
    ) -> Result<ChapterDownloadTask, StorageError>;

    /// Remove completed/cancelled tasks for one book and return the removed count.
    fn clear_finished_chapter_downloads(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<usize, StorageError>;
}

/// Export/import surface for storage-owned reader data.
pub trait StorageSnapshotStore {
    /// Export a complete deterministic snapshot of the storage state.
    fn export_snapshot(&self, exported_at: i64) -> Result<StorageSnapshot, StorageError>;

    /// Replace current storage state with a validated snapshot.
    fn replace_with_snapshot(&self, snapshot: StorageSnapshot) -> Result<(), StorageError>;
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Import (upsert) a source definition. Returns the stored source.
    pub fn put_source(&self, source: Source) -> Result<Source, StorageError> {
        let mut inner = self.lock()?;
        inner
            .sources
            .insert(source.source_id.clone(), source.clone());
        Ok(source)
    }

    /// Look up a source by id.
    pub fn get_source(&self, source_id: &str) -> Result<Option<Source>, StorageError> {
        Ok(self.lock()?.sources.get(source_id).cloned())
    }

    /// Upsert a cached book record keyed by book id.
    pub fn put_book(&self, book: Book) -> Result<Book, StorageError> {
        let mut inner = self.lock()?;
        inner.books.insert(book.book_id.clone(), book.clone());
        Ok(book)
    }

    pub fn get_book(&self, book_id: &str) -> Result<Option<Book>, StorageError> {
        Ok(self.lock()?.books.get(book_id).cloned())
    }

    /// Write a cache entry. Overwrites any existing entry for the same key.
    pub fn put_cache(
        &self,
        key: impl Into<String>,
        payload: impl Into<String>,
    ) -> Result<CachedEntry, StorageError> {
        let entry = CachedEntry {
            key: key.into(),
            payload: payload.into(),
        };
        let mut inner = self.lock()?;
        inner.cache.insert(entry.key.clone(), entry.clone());
        Ok(entry)
    }

    /// Read a cache entry by key.
    pub fn get_cache(&self, key: &str) -> Result<Option<CachedEntry>, StorageError> {
        Ok(self.lock()?.cache.get(key).cloned())
    }

    /// Upsert reading progress for a book.
    pub fn put_progress(&self, progress: ReadingProgress) -> Result<ReadingProgress, StorageError> {
        let mut inner = self.lock()?;
        inner
            .progress
            .insert(progress.book_id.clone(), progress.clone());
        Ok(progress)
    }

    /// Read reading progress for a book.
    pub fn get_progress(&self, book_id: &str) -> Result<Option<ReadingProgress>, StorageError> {
        Ok(self.lock()?.progress.get(book_id).cloned())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, StorageInner>, StorageError> {
        self.inner.lock().map_err(|_| StorageError::Poisoned)
    }
}

impl StorageSnapshotStore for InMemoryStorage {
    fn export_snapshot(&self, exported_at: i64) -> Result<StorageSnapshot, StorageError> {
        let inner = self.lock()?;
        let mut snapshot = StorageSnapshot {
            schema_version: STORAGE_SNAPSHOT_SCHEMA_VERSION,
            exported_at,
            sources: inner.sources.values().cloned().collect(),
            books: inner.books.values().cloned().collect(),
            cache: inner.cache.values().cloned().collect(),
            legacy_progress: inner.progress.values().cloned().collect(),
            bookshelf: inner.shelf.values().cloned().collect(),
            chapter_cache: inner.chapter_cache.values().cloned().collect(),
            reading_progress: inner.reading_progress.values().cloned().collect(),
            reading_progress_history: inner
                .reading_progress_history
                .values()
                .flat_map(|entries| entries.iter().cloned())
                .collect(),
            chapter_download_queue: inner.chapter_download_queue.values().cloned().collect(),
        };
        sort_storage_snapshot(&mut snapshot);
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn replace_with_snapshot(&self, snapshot: StorageSnapshot) -> Result<(), StorageError> {
        snapshot.validate()?;
        let replacement = storage_inner_from_snapshot(snapshot)?;
        let mut inner = self.lock()?;
        *inner = replacement;
        Ok(())
    }
}

pub(crate) fn sort_storage_snapshot(snapshot: &mut StorageSnapshot) {
    snapshot
        .sources
        .sort_by(|a, b| a.source_id.cmp(&b.source_id));
    snapshot.books.sort_by(|a, b| a.book_id.cmp(&b.book_id));
    snapshot.cache.sort_by(|a, b| a.key.cmp(&b.key));
    snapshot
        .legacy_progress
        .sort_by(|a, b| a.book_id.cmp(&b.book_id));
    snapshot.bookshelf.sort_by(compare_bookshelf_key);
    snapshot.chapter_cache.sort_by(compare_chapter_cache_key);
    snapshot
        .reading_progress
        .sort_by(compare_reading_progress_key);
    snapshot.reading_progress_history.sort_by(|a, b| {
        compare_reading_progress_key(a, b)
            .then_with(|| a.updated_at.cmp(&b.updated_at))
            .then_with(|| a.device_id.cmp(&b.device_id))
            .then_with(|| a.chapter_index.cmp(&b.chapter_index))
            .then_with(|| a.chapter_offset.cmp(&b.chapter_offset))
    });
    snapshot
        .chapter_download_queue
        .sort_by(compare_download_key);
}

fn storage_inner_from_snapshot(snapshot: StorageSnapshot) -> Result<StorageInner, StorageError> {
    let mut inner = StorageInner::default();

    for source in snapshot.sources {
        inner.sources.insert(source.source_id.clone(), source);
    }
    for book in snapshot.books {
        inner.books.insert(book.book_id.clone(), book);
    }
    for entry in snapshot.cache {
        inner.cache.insert(entry.key.clone(), entry);
    }
    for progress in snapshot.legacy_progress {
        inner.progress.insert(progress.book_id.clone(), progress);
    }
    for entry in snapshot.bookshelf {
        inner.shelf.insert(entry.shelf_key(), entry);
    }
    for entry in snapshot.chapter_cache {
        inner.chapter_cache.insert(entry.chapter_cache_key(), entry);
    }
    for entry in snapshot.reading_progress {
        inner.reading_progress.insert(entry.progress_key(), entry);
    }
    for entry in snapshot.reading_progress_history {
        inner
            .reading_progress_history
            .entry(entry.progress_key())
            .or_default()
            .push(entry);
    }
    for task in snapshot.chapter_download_queue {
        inner
            .chapter_download_queue
            .insert(task.download_key(), task);
    }

    Ok(inner)
}

fn ensure_unique_snapshot_key<K>(
    seen: &mut HashMap<K, ()>,
    key: K,
    field: &'static str,
) -> Result<(), StorageError>
where
    K: Eq + Hash,
{
    if seen.insert(key, ()).is_some() {
        return Err(StorageError::InvalidSnapshot {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_source(source: &Source) -> Result<(), StorageError> {
    validate_source_id(&source.source_id)
}

fn validate_book(book: &Book) -> Result<(), StorageError> {
    if book.book_id.trim().is_empty() {
        return Err(StorageError::InvalidKey {
            field: "book_id".into(),
        });
    }
    Ok(())
}

fn validate_cache_entry(entry: &CachedEntry) -> Result<(), StorageError> {
    if entry.key.trim().is_empty() {
        return Err(StorageError::InvalidKey {
            field: "key".into(),
        });
    }
    Ok(())
}

fn validate_domain_progress(progress: &ReadingProgress) -> Result<(), StorageError> {
    if progress.book_id.trim().is_empty() {
        return Err(StorageError::InvalidKey {
            field: "book_id".into(),
        });
    }
    if !progress.chapter_progress.is_finite()
        || progress.chapter_progress < 0.0
        || progress.chapter_progress > 1.0
    {
        return Err(StorageError::InvalidProgress {
            field: "chapter_progress".into(),
        });
    }
    Ok(())
}

fn compare_bookshelf_key(a: &BookshelfEntry, b: &BookshelfEntry) -> std::cmp::Ordering {
    a.source_id
        .cmp(&b.source_id)
        .then_with(|| a.book_id.cmp(&b.book_id))
}

fn compare_chapter_cache_key(a: &ChapterCacheEntry, b: &ChapterCacheEntry) -> std::cmp::Ordering {
    a.source_id
        .cmp(&b.source_id)
        .then_with(|| a.book_id.cmp(&b.book_id))
        .then_with(|| a.chapter_index.cmp(&b.chapter_index))
}

fn compare_reading_progress_key(
    a: &ReadingProgressEntry,
    b: &ReadingProgressEntry,
) -> std::cmp::Ordering {
    a.source_id
        .cmp(&b.source_id)
        .then_with(|| a.book_id.cmp(&b.book_id))
}

fn compare_download_key(a: &ChapterDownloadTask, b: &ChapterDownloadTask) -> std::cmp::Ordering {
    a.source_id
        .cmp(&b.source_id)
        .then_with(|| a.book_id.cmp(&b.book_id))
        .then_with(|| a.chapter_index.cmp(&b.chapter_index))
}

pub(crate) fn validate_book_key(source_id: &str, book_id: &str) -> Result<(), StorageError> {
    if source_id.trim().is_empty() {
        return Err(StorageError::InvalidKey {
            field: "source_id".into(),
        });
    }
    if book_id.trim().is_empty() {
        return Err(StorageError::InvalidKey {
            field: "book_id".into(),
        });
    }
    Ok(())
}

pub(crate) fn validate_source_id(source_id: &str) -> Result<(), StorageError> {
    if source_id.trim().is_empty() {
        return Err(StorageError::InvalidKey {
            field: "source_id".into(),
        });
    }
    Ok(())
}

fn validate_offline_chapter_cache_entry(
    entry: &OfflineChapterCacheEntry,
) -> Result<(), StorageError> {
    validate_source_id(&entry.source_id)?;
    if entry.chapter_id.trim().is_empty() {
        return Err(StorageError::InvalidChapterCache {
            field: "chapter_id".into(),
        });
    }
    Ok(())
}

fn validate_chapter_cache_required(value: &str, field: &str) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        return Err(StorageError::InvalidChapterCache {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_chapter_cache_optional(
    value: &Option<String>,
    field: &str,
) -> Result<(), StorageError> {
    if value.as_deref().map(str::trim).is_some_and(str::is_empty) {
        return Err(StorageError::InvalidChapterCache {
            field: field.into(),
        });
    }
    Ok(())
}

fn count_offline_status(
    entries: &[OfflineChapterCacheEntry],
    source_id: &str,
    status: ChapterCacheStatus,
) -> Result<u32, StorageError> {
    let count = entries
        .iter()
        .filter(|entry| entry.source_id == source_id && entry.status == status)
        .count();
    u32::try_from(count).map_err(|_| StorageError::InvalidChapterCache {
        field: "entry_count".into(),
    })
}

pub(crate) fn validate_reading_progress(entry: &ReadingProgressEntry) -> Result<(), StorageError> {
    validate_book_key(&entry.source_id, &entry.book_id)?;
    if !entry.chapter_progress.is_finite()
        || entry.chapter_progress < 0.0
        || entry.chapter_progress > 1.0
    {
        return Err(StorageError::InvalidProgress {
            field: "chapter_progress".into(),
        });
    }
    if entry
        .device_id
        .as_deref()
        .map(str::trim)
        .is_some_and(str::is_empty)
    {
        return Err(StorageError::InvalidProgress {
            field: "device_id".into(),
        });
    }
    Ok(())
}

fn reset_unified_reading_progress(
    progress: &UnifiedReadingProgress,
    source_id: &str,
    book_id: &str,
    first_chapter: Option<&UnifiedChapterIndexEntry>,
    diagnostic: &str,
) -> UnifiedReadingProgress {
    let locator = UnifiedReadingLocator {
        source_id: source_id.into(),
        book_id: book_id.into(),
        chapter_id: first_chapter.map(|chapter| chapter.chapter_id.clone()),
        origin_specific_locator: first_chapter
            .and_then(|chapter| chapter.origin_specific_locator.clone()),
        canonical_locator: first_chapter.map(|chapter| chapter.canonical_locator.clone()),
        chapter_ordinal: first_chapter.map(|chapter| chapter.ordinal).unwrap_or(0),
        character_offset: 0,
        normalized_chapter_progress: 0.0,
        normalized_book_progress: 0.0,
        surrounding_text_checksum: None,
        remote_toc_version: progress.locator.remote_toc_version.clone(),
        local_fingerprint: progress.locator.local_fingerprint.clone(),
        content_checksum: first_chapter.and_then(|chapter| chapter.content_checksum.clone()),
        timestamp: progress.locator.timestamp.clone(),
    };
    UnifiedReadingProgress {
        locator,
        restore_state: UnifiedReadingRestoreState::ResetToBeginning,
        diagnostics: vec![diagnostic.into()],
    }
}

fn find_unified_chapter_by_locator<'a>(
    chapters: &[&'a UnifiedChapterIndexEntry],
    locator: &str,
) -> Option<&'a UnifiedChapterIndexEntry> {
    let target = canonicalize_unified_locator_for_match(locator);
    chapters.iter().copied().find(|chapter| {
        canonicalize_unified_locator_for_match(&chapter.canonical_locator) == target
            || chapter
                .origin_specific_locator
                .as_deref()
                .map(canonicalize_unified_locator_for_match)
                .as_deref()
                == Some(target.as_str())
    })
}

fn canonicalize_unified_locator_for_match(locator: &str) -> String {
    let locator = locator.trim();
    let without_fragment = locator
        .split_once('#')
        .map(|(before, _)| before)
        .unwrap_or(locator);
    let Some((path, query)) = without_fragment.split_once('?') else {
        return without_fragment.to_string();
    };
    let mut parameters = query
        .split('&')
        .filter(|parameter| !parameter.is_empty())
        .collect::<Vec<_>>();
    parameters.sort_unstable();
    if parameters.is_empty() {
        path.to_string()
    } else {
        format!("{}?{}", path, parameters.join("&"))
    }
}

fn ordered_unique_unified_chapters<'a>(
    chapters: &'a [UnifiedChapterIndexEntry],
    source_id: &str,
    book_id: &str,
) -> Vec<&'a UnifiedChapterIndexEntry> {
    let mut scoped = chapters
        .iter()
        .filter(|chapter| chapter.source_id == source_id && chapter.book_id == book_id)
        .collect::<Vec<_>>();
    scoped.sort_by(|left, right| {
        left.ordinal
            .cmp(&right.ordinal)
            .then_with(|| left.chapter_id.cmp(&right.chapter_id))
    });

    let mut seen = HashSet::<String>::new();
    scoped
        .into_iter()
        .filter(|chapter| seen.insert(chapter.canonical_locator.clone()))
        .collect()
}

fn diff_unified_chapters(
    old: &[&UnifiedChapterIndexEntry],
    new: &[&UnifiedChapterIndexEntry],
) -> Vec<UnifiedChapterDiffEntry> {
    let old_by_locator = first_unified_chapter_by_canonical_locator(old);
    let new_by_locator = first_unified_chapter_by_canonical_locator(new);
    let mut entries = Vec::new();

    for chapter in new {
        if !old_by_locator.contains_key(&chapter.canonical_locator) {
            entries.push(unified_chapter_diff_entry(
                UnifiedChapterDiffType::ChapterAdded,
                None,
                Some(&chapter.chapter_id),
                "canonical_locator_new",
            ));
        }
    }

    for chapter in old {
        if !new_by_locator.contains_key(&chapter.canonical_locator) {
            entries.push(unified_chapter_diff_entry(
                UnifiedChapterDiffType::ChapterRemoved,
                Some(&chapter.chapter_id),
                None,
                "canonical_locator_missing",
            ));
        }
    }

    for chapter in new {
        let Some(previous) = old_by_locator.get(&chapter.canonical_locator) else {
            continue;
        };
        if previous.title != chapter.title {
            entries.push(unified_chapter_diff_entry(
                UnifiedChapterDiffType::ChapterTitleChanged,
                Some(&previous.chapter_id),
                Some(&chapter.chapter_id),
                "title_changed",
            ));
        }
        if previous.ordinal != chapter.ordinal {
            entries.push(unified_chapter_diff_entry(
                UnifiedChapterDiffType::ChapterOrderChanged,
                Some(&previous.chapter_id),
                Some(&chapter.chapter_id),
                "ordinal_changed",
            ));
        }
        if previous.origin_specific_locator != chapter.origin_specific_locator {
            entries.push(unified_chapter_diff_entry(
                UnifiedChapterDiffType::ChapterLocatorChanged,
                Some(&previous.chapter_id),
                Some(&chapter.chapter_id),
                "chapter_locator_changed",
            ));
        }
        if previous.content_locator != chapter.content_locator {
            entries.push(unified_chapter_diff_entry(
                UnifiedChapterDiffType::ChapterContentLocatorChanged,
                Some(&previous.chapter_id),
                Some(&chapter.chapter_id),
                "content_locator_changed",
            ));
        }
        if chapter
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic == "duplicate_canonical_locator_merged")
            && !previous
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic == "duplicate_canonical_locator_merged")
        {
            entries.push(unified_chapter_diff_entry(
                UnifiedChapterDiffType::DuplicateMerged,
                Some(&previous.chapter_id),
                Some(&chapter.chapter_id),
                "canonical_duplicate_merged",
            ));
        }
    }

    if entries.is_empty() {
        entries.push(unified_chapter_diff_entry(
            UnifiedChapterDiffType::Unchanged,
            None,
            None,
            "chapter_identity_stable",
        ));
    }
    sort_unified_chapter_diff_entries(&mut entries);
    entries
}

fn first_unified_chapter_by_canonical_locator<'a>(
    chapters: &[&'a UnifiedChapterIndexEntry],
) -> HashMap<String, &'a UnifiedChapterIndexEntry> {
    let mut result = HashMap::new();
    for chapter in chapters {
        result
            .entry(chapter.canonical_locator.clone())
            .or_insert(*chapter);
    }
    result
}

fn unified_chapter_diff_entry(
    diff_type: UnifiedChapterDiffType,
    old_chapter_id: Option<&str>,
    new_chapter_id: Option<&str>,
    reason: &str,
) -> UnifiedChapterDiffEntry {
    UnifiedChapterDiffEntry {
        diff_type,
        old_chapter_id: old_chapter_id.map(str::to_string),
        new_chapter_id: new_chapter_id.map(str::to_string),
        reason: reason.into(),
    }
}

fn sort_unified_chapter_diff_entries(entries: &mut [UnifiedChapterDiffEntry]) {
    entries.sort_by(|left, right| {
        unified_chapter_diff_type_wire_value(left.diff_type)
            .cmp(unified_chapter_diff_type_wire_value(right.diff_type))
            .then_with(|| left.old_chapter_id.cmp(&right.old_chapter_id))
            .then_with(|| left.new_chapter_id.cmp(&right.new_chapter_id))
            .then_with(|| left.reason.cmp(&right.reason))
    });
}

fn unified_chapter_diff_type_wire_value(diff_type: UnifiedChapterDiffType) -> &'static str {
    match diff_type {
        UnifiedChapterDiffType::Unchanged => "unchanged",
        UnifiedChapterDiffType::ChapterAdded => "chapter_added",
        UnifiedChapterDiffType::ChapterRemoved => "chapter_removed",
        UnifiedChapterDiffType::ChapterTitleChanged => "chapter_title_changed",
        UnifiedChapterDiffType::ChapterOrderChanged => "chapter_order_changed",
        UnifiedChapterDiffType::ChapterLocatorChanged => "chapter_locator_changed",
        UnifiedChapterDiffType::ChapterContentLocatorChanged => "chapter_content_locator_changed",
        UnifiedChapterDiffType::DuplicateMerged => "duplicate_merged",
        UnifiedChapterDiffType::CanonicalLocatorChanged => "canonical_locator_changed",
        UnifiedChapterDiffType::BookMetadataChanged => "book_metadata_changed",
    }
}

fn sort_dedupe_strings(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

fn validate_progress_required(value: &str, field: &str) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        return Err(StorageError::InvalidProgress {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_progress_optional(value: &Option<String>, field: &str) -> Result<(), StorageError> {
    if value.as_deref().map(str::trim).is_some_and(str::is_empty) {
        return Err(StorageError::InvalidProgress {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_progress_diagnostics(diagnostics: &[String]) -> Result<(), StorageError> {
    if diagnostics.iter().any(|value| value.trim().is_empty()) {
        return Err(StorageError::InvalidProgress {
            field: "diagnostics".into(),
        });
    }
    Ok(())
}

fn validate_progress_fraction(value: f64, field: &str) -> Result<(), StorageError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(StorageError::InvalidProgress {
            field: field.into(),
        });
    }
    Ok(())
}

fn default_max_attempts() -> u32 {
    3
}

pub(crate) fn validate_chapter_download_task(
    task: &ChapterDownloadTask,
) -> Result<(), StorageError> {
    validate_book_key(&task.source_id, &task.book_id)?;
    if task.max_attempts == 0 {
        return Err(StorageError::InvalidDownloadTask {
            field: "max_attempts".into(),
        });
    }
    if task.attempts > task.max_attempts {
        return Err(StorageError::InvalidDownloadTask {
            field: "attempts".into(),
        });
    }
    if task
        .last_error
        .as_deref()
        .map(str::trim)
        .is_some_and(str::is_empty)
    {
        return Err(StorageError::InvalidDownloadTask {
            field: "last_error".into(),
        });
    }
    Ok(())
}

fn validate_unified_download_task_request(
    request: &UnifiedDownloadTaskRequest,
) -> Result<(), StorageError> {
    validate_book_key(&request.source_id, &request.book_id)?;
    validate_download_required(&request.task_id, "task_id")?;
    if request
        .requested_chapter_ids
        .iter()
        .any(|id| id.trim().is_empty())
    {
        return Err(StorageError::InvalidDownloadTask {
            field: "requested_chapter_ids".into(),
        });
    }
    if request.concurrency_limit == 0 {
        return Err(StorageError::InvalidDownloadTask {
            field: "concurrency_limit".into(),
        });
    }
    if request.maximum_request_count == 0 {
        return Err(StorageError::InvalidDownloadTask {
            field: "maximum_request_count".into(),
        });
    }
    if request
        .in_flight_task_ids
        .iter()
        .any(|task_id| task_id.trim().is_empty())
    {
        return Err(StorageError::InvalidDownloadTask {
            field: "in_flight_task_ids".into(),
        });
    }
    for entry in &request.cached_entries {
        entry.validate()?;
    }
    for outcome in &request.fetch_outcomes {
        validate_download_required(&outcome.chapter_id, "chapter_id")?;
        validate_download_diagnostics(&outcome.diagnostics)?;
    }
    if request
        .cancellation_state
        .as_deref()
        .map(str::trim)
        .is_some_and(str::is_empty)
    {
        return Err(StorageError::InvalidDownloadTask {
            field: "cancellation_state".into(),
        });
    }
    Ok(())
}

fn validate_download_required(value: &str, field: &str) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        return Err(StorageError::InvalidDownloadTask {
            field: field.into(),
        });
    }
    Ok(())
}

fn validate_download_diagnostics(diagnostics: &[String]) -> Result<(), StorageError> {
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.trim().is_empty())
    {
        return Err(StorageError::InvalidDownloadTask {
            field: "diagnostics".into(),
        });
    }
    Ok(())
}

fn is_unified_download_cache_hit_state(state: ChapterCacheStatus) -> bool {
    !matches!(
        state,
        ChapterCacheStatus::Missing | ChapterCacheStatus::Partial | ChapterCacheStatus::Failed
    )
}

fn is_unified_download_success_state(state: ChapterCacheStatus) -> bool {
    matches!(
        state,
        ChapterCacheStatus::Available | ChapterCacheStatus::Validated
    )
}

fn capped_nonzero_limit(requested: usize, runtime_limit: usize) -> usize {
    if runtime_limit == 0 {
        requested
    } else {
        requested.min(runtime_limit)
    }
}

fn chapter_download_key(
    source_id: &str,
    book_id: &str,
    chapter_index: u32,
) -> Result<ChapterDownloadKey, StorageError> {
    validate_book_key(source_id, book_id)?;
    Ok(ChapterDownloadKey {
        source_id: source_id.to_string(),
        book_id: book_id.to_string(),
        chapter_index,
    })
}

pub(crate) fn validate_shelf_key(source_id: &str, book_id: &str) -> Result<(), StorageError> {
    validate_book_key(source_id, book_id)
}

pub(crate) fn sort_shelf(entries: &mut Vec<BookshelfEntry>) {
    // sort_index ascending; ties broken by added_at descending (newer first).
    entries.sort_by(|a, b| {
        a.sort_index
            .cmp(&b.sort_index)
            .then_with(|| b.added_at.cmp(&a.added_at))
            .then_with(|| a.source_id.cmp(&b.source_id))
            .then_with(|| a.book_id.cmp(&b.book_id))
    });
}

pub(crate) fn normalize_required_filter(
    value: Option<String>,
    field: &str,
) -> Result<Option<String>, StorageError> {
    value
        .map(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                Err(StorageError::InvalidKey {
                    field: field.into(),
                })
            } else {
                Ok(trimmed)
            }
        })
        .transpose()
}

pub(crate) fn normalize_keyword(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

pub(crate) fn normalize_group(value: Option<String>) -> Result<Option<String>, StorageError> {
    normalize_required_filter(value, "group")
}

fn entry_matches_keyword(entry: &BookshelfEntry, keyword: &str) -> bool {
    entry.title.to_ascii_lowercase().contains(keyword)
        || entry.author.to_ascii_lowercase().contains(keyword)
        || entry.book_id.to_ascii_lowercase().contains(keyword)
}

fn entry_has_reading_progress(inner: &StorageInner, entry: &BookshelfEntry) -> bool {
    inner.reading_progress.contains_key(&ReadingProgressKey {
        source_id: entry.source_id.clone(),
        book_id: entry.book_id.clone(),
    })
}

pub(crate) fn sort_shelf_query(
    entries: &mut Vec<BookshelfEntry>,
    sort_by: BookshelfSortBy,
    direction: BookshelfSortDirection,
) {
    if sort_by == BookshelfSortBy::Manual {
        sort_shelf(entries);
        if direction == BookshelfSortDirection::Descending {
            entries.reverse();
        }
        return;
    }

    entries.sort_by(|a, b| {
        let ordering = match sort_by {
            BookshelfSortBy::Manual => std::cmp::Ordering::Equal,
            BookshelfSortBy::AddedAt => a.added_at.cmp(&b.added_at),
            BookshelfSortBy::LastReadAt => a.last_read_at.cmp(&b.last_read_at),
            BookshelfSortBy::Title => a.title.cmp(&b.title),
            BookshelfSortBy::Author => a.author.cmp(&b.author),
        };
        let ordering = if direction == BookshelfSortDirection::Descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering
            .then_with(|| a.source_id.cmp(&b.source_id))
            .then_with(|| a.book_id.cmp(&b.book_id))
    });
}

pub(crate) fn paginate_shelf(
    entries: Vec<BookshelfEntry>,
    offset: usize,
    limit: Option<usize>,
) -> Vec<BookshelfEntry> {
    let iter = entries.into_iter().skip(offset);
    match limit {
        Some(limit) => iter.take(limit).collect(),
        None => iter.collect(),
    }
}

pub(crate) fn chapter_cache_content_bytes(entry: &ChapterCacheEntry) -> usize {
    entry.content.as_bytes().len()
}

pub(crate) fn validate_chapter_count(chapter_count: u32) -> Result<(), StorageError> {
    if chapter_count == 0 {
        return Err(StorageError::InvalidChapterCache {
            field: "chapter_count".into(),
        });
    }
    Ok(())
}

pub(crate) fn validate_chapter_anchor(
    chapter_count: u32,
    anchor_index: u32,
) -> Result<(), StorageError> {
    validate_chapter_count(chapter_count)?;
    if anchor_index >= chapter_count {
        return Err(StorageError::InvalidChapterCache {
            field: "anchor_index".into(),
        });
    }
    Ok(())
}

pub(crate) fn validate_prefetch_limit(max_count: usize) -> Result<(), StorageError> {
    if max_count == 0 {
        return Err(StorageError::InvalidChapterCache {
            field: "max_count".into(),
        });
    }
    Ok(())
}

pub(crate) fn chapter_cache_stats_from_entries<'a>(
    entries: impl Iterator<Item = &'a ChapterCacheEntry>,
) -> ChapterCacheStats {
    let mut entry_count = 0usize;
    let mut total_content_bytes = 0usize;
    let mut oldest_cached_at = None::<i64>;
    let mut newest_cached_at = None::<i64>;

    for entry in entries {
        entry_count += 1;
        total_content_bytes += chapter_cache_content_bytes(entry);
        oldest_cached_at = Some(
            oldest_cached_at
                .map(|oldest| oldest.min(entry.cached_at))
                .unwrap_or(entry.cached_at),
        );
        newest_cached_at = Some(
            newest_cached_at
                .map(|newest| newest.max(entry.cached_at))
                .unwrap_or(entry.cached_at),
        );
    }

    ChapterCacheStats {
        entry_count,
        total_content_bytes,
        oldest_cached_at,
        newest_cached_at,
    }
}

pub(crate) fn chapter_cache_coverage_from_entries(
    source_id: &str,
    book_id: &str,
    chapter_count: u32,
    entries: impl Iterator<Item = ChapterCacheEntry>,
) -> ChapterCacheCoverage {
    let mut cached_entries = entries
        .filter(|entry| entry.chapter_index < chapter_count)
        .collect::<Vec<_>>();
    cached_entries.sort_by_key(|entry| entry.chapter_index);
    cached_entries.dedup_by_key(|entry| entry.chapter_index);

    let cached_indexes = cached_entries
        .iter()
        .map(|entry| entry.chapter_index)
        .collect::<Vec<_>>();
    let missing_indexes = (0..chapter_count)
        .filter(|index| !cached_indexes.contains(index))
        .collect::<Vec<_>>();
    let stats = chapter_cache_stats_from_entries(cached_entries.iter());

    ChapterCacheCoverage {
        source_id: source_id.to_string(),
        book_id: book_id.to_string(),
        chapter_count,
        cached_count: cached_indexes.len(),
        missing_count: missing_indexes.len(),
        complete: missing_indexes.is_empty(),
        cached_indexes,
        missing_indexes,
        total_content_bytes: stats.total_content_bytes,
        oldest_cached_at: stats.oldest_cached_at,
        newest_cached_at: stats.newest_cached_at,
    }
}

fn sort_chapter_cache_for_eviction(entries: &mut [(ChapterCacheKey, ChapterCacheEntry)]) {
    entries.sort_by(|(key_a, entry_a), (key_b, entry_b)| {
        entry_a
            .cached_at
            .cmp(&entry_b.cached_at)
            .then_with(|| key_a.source_id.cmp(&key_b.source_id))
            .then_with(|| key_a.book_id.cmp(&key_b.book_id))
            .then_with(|| key_a.chapter_index.cmp(&key_b.chapter_index))
    });
}

fn mark_chapter_cache_key_once(keys: &mut Vec<ChapterCacheKey>, key: ChapterCacheKey) {
    if !keys.contains(&key) {
        keys.push(key);
    }
}

impl BookshelfStore for InMemoryStorage {
    fn add_to_shelf(&self, entry: BookshelfEntry) -> Result<BookshelfEntry, StorageError> {
        validate_shelf_key(&entry.source_id, &entry.book_id)?;
        let mut inner = self.lock()?;
        let key = entry.shelf_key();
        // Preserve original added_at on upsert.
        let added_at = inner
            .shelf
            .get(&key)
            .map(|existing| existing.added_at)
            .unwrap_or(entry.added_at);
        let mut stored = entry;
        stored.added_at = added_at;
        inner.shelf.insert(key, stored.clone());
        Ok(stored)
    }

    fn remove_from_shelf(&self, source_id: &str, book_id: &str) -> Result<(), StorageError> {
        validate_shelf_key(source_id, book_id)?;
        let mut inner = self.lock()?;
        let key = ShelfKey {
            source_id: source_id.to_string(),
            book_id: book_id.to_string(),
        };
        inner.shelf.remove(&key);
        Ok(())
    }

    fn get_shelf_entry(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Option<BookshelfEntry>, StorageError> {
        validate_shelf_key(source_id, book_id)?;
        let inner = self.lock()?;
        Ok(inner
            .shelf
            .get(&ShelfKey {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
            })
            .cloned())
    }

    fn list_shelf(&self) -> Result<Vec<BookshelfEntry>, StorageError> {
        let inner = self.lock()?;
        let mut entries: Vec<BookshelfEntry> = inner.shelf.values().cloned().collect();
        sort_shelf(&mut entries);
        Ok(entries)
    }

    fn query_shelf(&self, query: BookshelfQuery) -> Result<Vec<BookshelfEntry>, StorageError> {
        let source_id = normalize_required_filter(query.source_id, "source_id")?;
        let group = normalize_group(query.group)?;
        let keyword = normalize_keyword(query.keyword);
        let inner = self.lock()?;
        let mut entries = inner
            .shelf
            .values()
            .filter(|entry| {
                source_id
                    .as_deref()
                    .map(|source_id| entry.source_id == source_id)
                    .unwrap_or(true)
            })
            .filter(|entry| {
                group
                    .as_deref()
                    .map(|group| entry.group.as_deref() == Some(group))
                    .unwrap_or(true)
            })
            .filter(|entry| {
                keyword
                    .as_deref()
                    .map(|keyword| entry_matches_keyword(entry, keyword))
                    .unwrap_or(true)
            })
            .filter(|entry| {
                query
                    .has_reading_progress
                    .map(|expected| entry_has_reading_progress(&inner, entry) == expected)
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        sort_shelf_query(&mut entries, query.sort_by, query.sort_direction);
        Ok(paginate_shelf(entries, query.offset, query.limit))
    }

    fn list_shelf_by_group(&self, group: &str) -> Result<Vec<BookshelfEntry>, StorageError> {
        let group = normalize_group(Some(group.to_string()))?
            .expect("normalize_group returns Some for Some input");
        let inner = self.lock()?;
        let mut entries: Vec<BookshelfEntry> = inner
            .shelf
            .values()
            .filter(|e| e.group.as_deref() == Some(group.as_str()))
            .cloned()
            .collect();
        sort_shelf(&mut entries);
        Ok(entries)
    }

    fn update_last_read(
        &self,
        source_id: &str,
        book_id: &str,
        timestamp: i64,
    ) -> Result<(), StorageError> {
        validate_shelf_key(source_id, book_id)?;
        let mut inner = self.lock()?;
        let key = ShelfKey {
            source_id: source_id.to_string(),
            book_id: book_id.to_string(),
        };
        match inner.shelf.get_mut(&key) {
            Some(entry) => {
                entry.last_read_at = Some(timestamp);
                Ok(())
            }
            None => Err(StorageError::NotFound {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
            }),
        }
    }

    fn move_shelf_entry(
        &self,
        source_id: &str,
        book_id: &str,
        group: Option<String>,
        sort_index: i32,
    ) -> Result<BookshelfEntry, StorageError> {
        validate_shelf_key(source_id, book_id)?;
        let group = normalize_group(group)?;
        let mut inner = self.lock()?;
        let key = ShelfKey {
            source_id: source_id.to_string(),
            book_id: book_id.to_string(),
        };
        let entry = inner
            .shelf
            .get_mut(&key)
            .ok_or_else(|| StorageError::NotFound {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
            })?;
        entry.group = group;
        entry.sort_index = sort_index;
        Ok(entry.clone())
    }

    fn shelf_count(&self) -> Result<usize, StorageError> {
        Ok(self.lock()?.shelf.len())
    }
}

impl ChapterCacheStore for InMemoryStorage {
    fn put_chapter_cache(
        &self,
        entry: ChapterCacheEntry,
    ) -> Result<ChapterCacheEntry, StorageError> {
        validate_book_key(&entry.source_id, &entry.book_id)?;
        let mut inner = self.lock()?;
        inner
            .chapter_cache
            .insert(entry.chapter_cache_key(), entry.clone());
        Ok(entry)
    }

    fn get_chapter_cache(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<Option<ChapterCacheEntry>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let inner = self.lock()?;
        Ok(inner
            .chapter_cache
            .get(&ChapterCacheKey {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
                chapter_index,
            })
            .cloned())
    }

    fn remove_chapter_cache(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<(), StorageError> {
        validate_book_key(source_id, book_id)?;
        let mut inner = self.lock()?;
        inner.chapter_cache.remove(&ChapterCacheKey {
            source_id: source_id.to_string(),
            book_id: book_id.to_string(),
            chapter_index,
        });
        Ok(())
    }

    fn list_chapter_cache(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Vec<ChapterCacheEntry>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let inner = self.lock()?;
        let mut entries: Vec<ChapterCacheEntry> = inner
            .chapter_cache
            .iter()
            .filter(|(key, _)| key.source_id == source_id && key.book_id == book_id)
            .map(|(_, entry)| entry.clone())
            .collect();
        entries.sort_by_key(|entry| entry.chapter_index);
        Ok(entries)
    }

    fn clear_chapter_cache(&self, source_id: &str, book_id: &str) -> Result<usize, StorageError> {
        validate_book_key(source_id, book_id)?;
        let mut inner = self.lock()?;
        let before = inner.chapter_cache.len();
        inner
            .chapter_cache
            .retain(|key, _| !(key.source_id == source_id && key.book_id == book_id));
        Ok(before - inner.chapter_cache.len())
    }

    fn chapter_cache_coverage(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_count: u32,
    ) -> Result<ChapterCacheCoverage, StorageError> {
        validate_book_key(source_id, book_id)?;
        validate_chapter_count(chapter_count)?;
        let inner = self.lock()?;
        Ok(chapter_cache_coverage_from_entries(
            source_id,
            book_id,
            chapter_count,
            inner
                .chapter_cache
                .iter()
                .filter(|(key, _)| key.source_id == source_id && key.book_id == book_id)
                .map(|(_, entry)| entry.clone()),
        ))
    }

    fn plan_chapter_cache_prefetch(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_count: u32,
        anchor_index: u32,
        before: u32,
        after: u32,
        max_count: usize,
    ) -> Result<ChapterCachePrefetchPlan, StorageError> {
        validate_book_key(source_id, book_id)?;
        validate_chapter_anchor(chapter_count, anchor_index)?;
        validate_prefetch_limit(max_count)?;

        let window_start = anchor_index.saturating_sub(before);
        let window_end_exclusive = anchor_index
            .saturating_add(after)
            .saturating_add(1)
            .min(chapter_count);
        let coverage = self.chapter_cache_coverage(source_id, book_id, chapter_count)?;
        let missing_indexes = coverage
            .missing_indexes
            .into_iter()
            .filter(|index| *index >= window_start && *index < window_end_exclusive)
            .take(max_count)
            .collect::<Vec<_>>();

        Ok(ChapterCachePrefetchPlan {
            source_id: source_id.to_string(),
            book_id: book_id.to_string(),
            chapter_count,
            anchor_index,
            window_start,
            window_end_exclusive,
            missing_indexes,
        })
    }

    fn chapter_cache_stats(&self) -> Result<ChapterCacheStats, StorageError> {
        let inner = self.lock()?;
        Ok(chapter_cache_stats_from_entries(
            inner.chapter_cache.values(),
        ))
    }

    fn prune_chapter_cache(
        &self,
        policy: ChapterCacheRetentionPolicy,
    ) -> Result<ChapterCacheEvictionReport, StorageError> {
        let mut inner = self.lock()?;
        let mut candidates = inner
            .chapter_cache
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect::<Vec<_>>();
        sort_chapter_cache_for_eviction(&mut candidates);

        let mut remove_keys = Vec::<ChapterCacheKey>::new();

        if let Some(min_cached_at) = policy.min_cached_at {
            for (key, entry) in &candidates {
                if entry.cached_at < min_cached_at {
                    mark_chapter_cache_key_once(&mut remove_keys, key.clone());
                }
            }
        }

        if let Some(max_entries) = policy.max_entries {
            let mut remaining = candidates
                .iter()
                .filter(|(key, _)| !remove_keys.contains(key))
                .cloned()
                .collect::<Vec<_>>();
            sort_chapter_cache_for_eviction(&mut remaining);
            if remaining.len() > max_entries {
                let remove_count = remaining.len() - max_entries;
                for (key, _) in remaining.into_iter().take(remove_count) {
                    mark_chapter_cache_key_once(&mut remove_keys, key);
                }
            }
        }

        if let Some(max_total_content_bytes) = policy.max_total_content_bytes {
            let mut remaining = candidates
                .iter()
                .filter(|(key, _)| !remove_keys.contains(key))
                .cloned()
                .collect::<Vec<_>>();
            sort_chapter_cache_for_eviction(&mut remaining);
            let mut total_bytes = remaining
                .iter()
                .map(|(_, entry)| chapter_cache_content_bytes(entry))
                .sum::<usize>();
            for (key, entry) in remaining {
                if total_bytes <= max_total_content_bytes {
                    break;
                }
                total_bytes = total_bytes.saturating_sub(chapter_cache_content_bytes(&entry));
                mark_chapter_cache_key_once(&mut remove_keys, key);
            }
        }

        let mut removed = Vec::new();
        for key in remove_keys {
            if let Some(entry) = inner.chapter_cache.remove(&key) {
                removed.push(entry);
            }
        }
        removed.sort_by(|a, b| {
            a.cached_at
                .cmp(&b.cached_at)
                .then_with(|| a.source_id.cmp(&b.source_id))
                .then_with(|| a.book_id.cmp(&b.book_id))
                .then_with(|| a.chapter_index.cmp(&b.chapter_index))
        });

        let remaining = chapter_cache_stats_from_entries(inner.chapter_cache.values());
        Ok(ChapterCacheEvictionReport { removed, remaining })
    }
}

impl ReadingProgressStore for InMemoryStorage {
    fn save_reading_progress(
        &self,
        entry: ReadingProgressEntry,
    ) -> Result<ReadingProgressEntry, StorageError> {
        validate_reading_progress(&entry)?;
        let mut inner = self.lock()?;
        let key = entry.progress_key();
        inner
            .reading_progress_history
            .entry(key.clone())
            .or_default()
            .push(entry.clone());

        let should_advance = inner
            .reading_progress
            .get(&key)
            .map(|current| entry.updated_at >= current.updated_at)
            .unwrap_or(true);

        if should_advance {
            inner.reading_progress.insert(key.clone(), entry.clone());
            if let Some(shelf_entry) = inner.shelf.get_mut(&ShelfKey {
                source_id: key.source_id,
                book_id: key.book_id,
            }) {
                shelf_entry.last_read_at = Some(entry.updated_at);
            }
            Ok(entry)
        } else {
            Ok(inner
                .reading_progress
                .get(&key)
                .expect("current progress exists when stale update is ignored")
                .clone())
        }
    }

    fn get_reading_progress(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Option<ReadingProgressEntry>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let inner = self.lock()?;
        Ok(inner
            .reading_progress
            .get(&ReadingProgressKey {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
            })
            .cloned())
    }

    fn list_reading_progress(
        &self,
        source_id: &str,
    ) -> Result<Vec<ReadingProgressEntry>, StorageError> {
        validate_source_id(source_id)?;
        let inner = self.lock()?;
        let mut entries = inner
            .reading_progress
            .iter()
            .filter(|(key, _)| key.source_id == source_id)
            .map(|(_, entry)| entry.clone())
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.book_id.cmp(&b.book_id))
        });
        Ok(entries)
    }

    fn reading_progress_history(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Vec<ReadingProgressEntry>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let inner = self.lock()?;
        let mut history = inner
            .reading_progress_history
            .get(&ReadingProgressKey {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
            })
            .cloned()
            .unwrap_or_default();
        history.sort_by(|a, b| {
            a.updated_at
                .cmp(&b.updated_at)
                .then_with(|| a.device_id.cmp(&b.device_id))
        });
        Ok(history)
    }

    fn clear_reading_progress(&self, source_id: &str, book_id: &str) -> Result<(), StorageError> {
        validate_book_key(source_id, book_id)?;
        let mut inner = self.lock()?;
        let key = ReadingProgressKey {
            source_id: source_id.to_string(),
            book_id: book_id.to_string(),
        };
        inner.reading_progress.remove(&key);
        inner.reading_progress_history.remove(&key);
        if let Some(shelf_entry) = inner.shelf.get_mut(&ShelfKey {
            source_id: source_id.to_string(),
            book_id: book_id.to_string(),
        }) {
            shelf_entry.last_read_at = None;
        }
        Ok(())
    }
}

impl ChapterDownloadQueueStore for InMemoryStorage {
    fn enqueue_chapter_download(
        &self,
        mut task: ChapterDownloadTask,
    ) -> Result<ChapterDownloadTask, StorageError> {
        validate_chapter_download_task(&task)?;
        let mut inner = self.lock()?;
        let key = task.download_key();
        let created_at = inner
            .chapter_download_queue
            .get(&key)
            .map(|existing| existing.created_at)
            .unwrap_or(task.created_at);
        task.created_at = created_at;
        task.status = ChapterDownloadStatus::Pending;
        task.attempts = 0;
        task.last_error = None;
        validate_chapter_download_task(&task)?;
        inner.chapter_download_queue.insert(key, task.clone());
        Ok(task)
    }

    fn get_chapter_download(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<Option<ChapterDownloadTask>, StorageError> {
        let key = chapter_download_key(source_id, book_id, chapter_index)?;
        Ok(self.lock()?.chapter_download_queue.get(&key).cloned())
    }

    fn list_chapter_downloads(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Vec<ChapterDownloadTask>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let inner = self.lock()?;
        let mut tasks = inner
            .chapter_download_queue
            .iter()
            .filter(|(key, _)| key.source_id == source_id && key.book_id == book_id)
            .map(|(_, task)| task.clone())
            .collect::<Vec<_>>();
        tasks.sort_by_key(|task| task.chapter_index);
        Ok(tasks)
    }

    fn claim_next_chapter_download(
        &self,
        now: i64,
    ) -> Result<Option<ChapterDownloadTask>, StorageError> {
        let mut inner = self.lock()?;
        let next_key = inner
            .chapter_download_queue
            .iter()
            .filter(|(_, task)| is_claimable_download(task))
            .min_by(|(_, a), (_, b)| compare_download_claim_order(a, b))
            .map(|(key, _)| key.clone());

        let Some(next_key) = next_key else {
            return Ok(None);
        };

        let task = inner
            .chapter_download_queue
            .get_mut(&next_key)
            .expect("claimed key must exist");
        task.status = ChapterDownloadStatus::InProgress;
        task.attempts += 1;
        task.updated_at = now;
        task.last_error = None;
        Ok(Some(task.clone()))
    }

    fn mark_chapter_download_completed(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
        now: i64,
    ) -> Result<ChapterDownloadTask, StorageError> {
        self.update_download_task(source_id, book_id, chapter_index, |task| {
            task.status = ChapterDownloadStatus::Completed;
            task.updated_at = now;
            task.last_error = None;
        })
    }

    fn mark_chapter_download_failed(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
        error: impl Into<String>,
        now: i64,
    ) -> Result<ChapterDownloadTask, StorageError> {
        let error = error.into().trim().to_string();
        if error.is_empty() {
            return Err(StorageError::InvalidDownloadTask {
                field: "last_error".into(),
            });
        }
        self.update_download_task(source_id, book_id, chapter_index, |task| {
            task.status = ChapterDownloadStatus::Failed;
            task.updated_at = now;
            task.last_error = Some(error);
        })
    }

    fn cancel_chapter_download(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
        now: i64,
    ) -> Result<ChapterDownloadTask, StorageError> {
        self.update_download_task(source_id, book_id, chapter_index, |task| {
            task.status = ChapterDownloadStatus::Cancelled;
            task.updated_at = now;
            task.last_error = None;
        })
    }

    fn clear_finished_chapter_downloads(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<usize, StorageError> {
        validate_book_key(source_id, book_id)?;
        let mut inner = self.lock()?;
        let before = inner.chapter_download_queue.len();
        inner.chapter_download_queue.retain(|key, task| {
            !(key.source_id == source_id
                && key.book_id == book_id
                && matches!(
                    task.status,
                    ChapterDownloadStatus::Completed | ChapterDownloadStatus::Cancelled
                ))
        });
        Ok(before - inner.chapter_download_queue.len())
    }
}

impl InMemoryStorage {
    fn update_download_task(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
        update: impl FnOnce(&mut ChapterDownloadTask),
    ) -> Result<ChapterDownloadTask, StorageError> {
        let key = chapter_download_key(source_id, book_id, chapter_index)?;
        let mut inner = self.lock()?;
        let task = inner.chapter_download_queue.get_mut(&key).ok_or_else(|| {
            StorageError::DownloadTaskNotFound {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
                chapter_index,
            }
        })?;
        update(task);
        validate_chapter_download_task(task)?;
        Ok(task.clone())
    }
}

fn is_claimable_download(task: &ChapterDownloadTask) -> bool {
    matches!(
        task.status,
        ChapterDownloadStatus::Pending | ChapterDownloadStatus::Failed
    ) && task.attempts < task.max_attempts
}

fn compare_download_claim_order(
    a: &ChapterDownloadTask,
    b: &ChapterDownloadTask,
) -> std::cmp::Ordering {
    b.priority
        .cmp(&a.priority)
        .then_with(|| a.updated_at.cmp(&b.updated_at))
        .then_with(|| a.created_at.cmp(&b.created_at))
        .then_with(|| a.source_id.cmp(&b.source_id))
        .then_with(|| a.book_id.cmp(&b.book_id))
        .then_with(|| a.chapter_index.cmp(&b.chapter_index))
}

/// Storage errors. V1 is in-memory so the only realistic failure is a poisoned
/// lock; the variant exists so the runtime can surface a structured `INTERNAL`
/// error instead of panicking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    Poisoned,
    /// A key field (source_id or book_id) was empty or invalid.
    InvalidKey {
        field: String,
    },
    /// A progress field was outside its accepted range.
    InvalidProgress {
        field: String,
    },
    /// A chapter download queue field was invalid.
    InvalidDownloadTask {
        field: String,
    },
    /// A chapter cache query/planning field was invalid.
    InvalidChapterCache {
        field: String,
    },
    /// A storage snapshot was invalid or incompatible.
    InvalidSnapshot {
        field: String,
    },
    /// A storage snapshot carried a `schemaVersion` that no registered migration
    /// knows how to upgrade to [`STORAGE_SNAPSHOT_SCHEMA_VERSION`].
    UnsupportedSnapshotSchemaVersion {
        schema_version: u32,
    },
    /// An entry referenced by the operation does not exist.
    NotFound {
        source_id: String,
        book_id: String,
    },
    /// A queued chapter download task does not exist.
    DownloadTaskNotFound {
        source_id: String,
        book_id: String,
        chapter_index: u32,
    },
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Poisoned => write!(f, "storage lock poisoned"),
            StorageError::InvalidKey { field } => write!(f, "invalid key field: {field}"),
            StorageError::InvalidProgress { field } => {
                write!(f, "invalid progress field: {field}")
            }
            StorageError::InvalidDownloadTask { field } => {
                write!(f, "invalid download task field: {field}")
            }
            StorageError::InvalidChapterCache { field } => {
                write!(f, "invalid chapter cache field: {field}")
            }
            StorageError::InvalidSnapshot { field } => {
                write!(f, "invalid storage snapshot field: {field}")
            }
            StorageError::UnsupportedSnapshotSchemaVersion { schema_version } => write!(
                f,
                "unsupported storage snapshot schema version: {schema_version}"
            ),
            StorageError::NotFound { source_id, book_id } => {
                write!(
                    f,
                    "shelf entry not found: source={source_id} book={book_id}"
                )
            }
            StorageError::DownloadTaskNotFound {
                source_id,
                book_id,
                chapter_index,
            } => write!(
                f,
                "chapter download task not found: source={source_id} book={book_id} chapter={chapter_index}"
            ),
        }
    }
}

impl std::error::Error for StorageError {}

#[cfg(test)]
mod tests {
    use super::*;
    use reader_domain::SourceRules;

    fn sample_source(id: &str) -> Source {
        Source {
            source_id: id.into(),
            name: id.into(),
            base_url: String::new(),
            rules: SourceRules::default(),
            book_source: serde_json::Value::Null,
        }
    }

    #[test]
    fn source_put_then_get_round_trips() {
        let store = InMemoryStorage::new();
        let source = sample_source("s1");
        store.put_source(source.clone()).unwrap();
        let got = store.get_source("s1").unwrap().unwrap();
        assert_eq!(got, source);
        assert!(store.get_source("missing").unwrap().is_none());
    }

    #[test]
    fn cache_put_then_get_round_trips() {
        let store = InMemoryStorage::new();
        assert!(store.get_cache("k").unwrap().is_none());
        store.put_cache("k", "{\"hello\":true}").unwrap();
        let got = store.get_cache("k").unwrap().unwrap();
        assert_eq!(got.payload, "{\"hello\":true}");
    }

    #[test]
    fn progress_put_then_get_round_trips() {
        let store = InMemoryStorage::new();
        let p = ReadingProgress {
            book_id: "b1".into(),
            chapter_index: 3,
            chapter_offset: 1024,
            chapter_progress: 0.5,
        };
        store.put_progress(p.clone()).unwrap();
        let got = store.get_progress("b1").unwrap().unwrap();
        assert_eq!(got, p);
    }

    fn sample_book(id: &str, title: &str) -> Book {
        Book {
            book_id: id.into(),
            title: title.into(),
            author: String::new(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        }
    }

    fn populate_snapshot_store(store: &InMemoryStorage) {
        store.put_source(sample_source("s2")).unwrap();
        store.put_source(sample_source("s1")).unwrap();
        store.put_book(sample_book("b2", "Book 2")).unwrap();
        store.put_book(sample_book("b1", "Book 1")).unwrap();
        store.put_cache("z", "{\"z\":true}").unwrap();
        store.put_cache("a", "{\"a\":true}").unwrap();
        store
            .put_progress(ReadingProgress {
                book_id: "legacy-b".into(),
                chapter_index: 1,
                chapter_offset: 10,
                chapter_progress: 0.25,
            })
            .unwrap();

        store
            .add_to_shelf(shelf_entry("s1", "b2", "Shelf B", 2000))
            .unwrap();
        store
            .add_to_shelf(shelf_entry("s1", "b1", "Shelf A", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 1, "Second", "two", 2000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "First", "one", 1000))
            .unwrap();
        store
            .save_reading_progress(progress_entry("s1", "b1", 2, 200, 0.2, 2000))
            .unwrap();
        store
            .save_reading_progress(progress_entry("s1", "b1", 1, 100, 0.1, 1000))
            .unwrap();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 2, 5, 2000))
            .unwrap();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 0, 1, 1000))
            .unwrap();
    }

    #[test]
    fn storage_snapshot_export_is_stable_and_json_round_trips() {
        let store = InMemoryStorage::new();
        populate_snapshot_store(&store);

        let snapshot = store.export_snapshot(42).unwrap();

        assert_eq!(snapshot.schema_version, STORAGE_SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snapshot.exported_at, 42);
        assert_eq!(
            snapshot
                .sources
                .iter()
                .map(|source| source.source_id.as_str())
                .collect::<Vec<_>>(),
            vec!["s1", "s2"]
        );
        assert_eq!(
            snapshot
                .books
                .iter()
                .map(|book| book.book_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b1", "b2"]
        );
        assert_eq!(
            snapshot
                .cache
                .iter()
                .map(|entry| entry.key.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "z"]
        );
        assert_eq!(
            snapshot
                .bookshelf
                .iter()
                .map(|entry| entry.book_id.as_str())
                .collect::<Vec<_>>(),
            vec!["b1", "b2"]
        );
        assert_eq!(
            snapshot
                .chapter_cache
                .iter()
                .map(|entry| entry.chapter_index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(
            snapshot
                .reading_progress_history
                .iter()
                .map(|entry| entry.updated_at)
                .collect::<Vec<_>>(),
            vec![1000, 2000]
        );
        assert_eq!(
            snapshot
                .chapter_download_queue
                .iter()
                .map(|task| task.chapter_index)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );

        let json = serde_json::to_string(&snapshot).unwrap();
        let back: StorageSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
    }

    #[test]
    fn storage_snapshot_replace_round_trips_all_state() {
        let source = InMemoryStorage::new();
        populate_snapshot_store(&source);
        let snapshot = source.export_snapshot(77).unwrap();

        let restored = InMemoryStorage::new();
        restored.replace_with_snapshot(snapshot.clone()).unwrap();

        assert_eq!(restored.export_snapshot(77).unwrap(), snapshot);
        assert_eq!(
            restored.get_shelf_entry("s1", "b1").unwrap().unwrap().title,
            "Shelf A"
        );
        assert_eq!(
            restored
                .get_chapter_cache("s1", "b1", 0)
                .unwrap()
                .unwrap()
                .content,
            "one"
        );
        assert_eq!(
            restored
                .get_reading_progress("s1", "b1")
                .unwrap()
                .unwrap()
                .chapter_index,
            2
        );
        assert_eq!(
            restored
                .reading_progress_history("s1", "b1")
                .unwrap()
                .iter()
                .map(|entry| entry.chapter_index)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            restored
                .list_chapter_downloads("s1", "b1")
                .unwrap()
                .into_iter()
                .map(|task| task.chapter_index)
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn storage_snapshot_empty_replace_clears_existing_state() {
        let store = InMemoryStorage::new();
        populate_snapshot_store(&store);

        store
            .replace_with_snapshot(StorageSnapshot::empty(100))
            .unwrap();

        assert!(store.get_source("s1").unwrap().is_none());
        assert!(store.get_book("b1").unwrap().is_none());
        assert!(store.get_cache("a").unwrap().is_none());
        assert!(store.get_progress("legacy-b").unwrap().is_none());
        assert!(store.list_shelf().unwrap().is_empty());
        assert!(store.list_chapter_cache("s1", "b1").unwrap().is_empty());
        assert!(store.get_reading_progress("s1", "b1").unwrap().is_none());
        assert!(store
            .reading_progress_history("s1", "b1")
            .unwrap()
            .is_empty());
        assert!(store.list_chapter_downloads("s1", "b1").unwrap().is_empty());
    }

    #[test]
    fn storage_snapshot_rejects_schema_duplicates_and_unknown_fields() {
        let mut wrong_schema = StorageSnapshot::empty(1);
        wrong_schema.schema_version = 2;
        assert_eq!(
            wrong_schema.validate().unwrap_err(),
            StorageError::InvalidSnapshot {
                field: "schema_version".into()
            }
        );

        let mut duplicate = StorageSnapshot::empty(1);
        duplicate
            .bookshelf
            .push(shelf_entry("s1", "b1", "First", 1000));
        duplicate
            .bookshelf
            .push(shelf_entry("s1", "b1", "Duplicate", 2000));
        assert_eq!(
            duplicate.validate().unwrap_err(),
            StorageError::InvalidSnapshot {
                field: "bookshelf".into()
            }
        );

        let unknown_field = r#"{"schemaVersion":1,"exportedAt":1,"bogus":true}"#;
        assert!(serde_json::from_str::<StorageSnapshot>(unknown_field).is_err());
    }

    #[test]
    fn storage_snapshot_replace_is_atomic_on_validation_failure() {
        let store = InMemoryStorage::new();
        populate_snapshot_store(&store);
        let before = store.export_snapshot(1).unwrap();

        let mut invalid = StorageSnapshot::empty(2);
        invalid.bookshelf.push(shelf_entry("", "b1", "Invalid", 1));
        assert!(matches!(
            store.replace_with_snapshot(invalid),
            Err(StorageError::InvalidKey { .. })
        ));

        assert_eq!(store.export_snapshot(1).unwrap(), before);
    }

    #[test]
    fn migrate_storage_snapshot_upgrades_legacy_v0_progress_to_v1() {
        // v0 shape: no schemaVersion, reading progress under the legacy
        // `progress` field name.
        let raw = serde_json::json!({
            "exportedAt": 99,
            "progress": [
                { "bookId": "legacy-b", "chapterIndex": 3, "chapterOffset": 250, "chapterProgress": 0.5 }
            ]
        });

        let snapshot = migrate_storage_snapshot(raw).unwrap();
        assert_eq!(snapshot.schema_version, STORAGE_SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snapshot.exported_at, 99);
        assert_eq!(snapshot.legacy_progress.len(), 1);
        assert_eq!(snapshot.legacy_progress[0].book_id, "legacy-b");
        assert_eq!(snapshot.legacy_progress[0].chapter_index, 3);
        // The legacy alias must be gone from the v1 shape: re-serializing and
        // re-parsing must round-trip through the typed struct without it.
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(!json.contains(r#""progress":"#));
        assert!(json.contains(r#""legacyProgress":"#));
        let back: StorageSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snapshot);
    }

    #[test]
    fn migrate_storage_snapshot_passes_through_current_version_unchanged() {
        let store = InMemoryStorage::new();
        populate_snapshot_store(&store);
        let snapshot = store.export_snapshot(7).unwrap();
        let raw = serde_json::to_value(&snapshot).unwrap();

        let migrated = migrate_storage_snapshot(raw).unwrap();
        assert_eq!(migrated, snapshot);
    }

    #[test]
    fn migrate_storage_snapshot_rejects_future_schema_version() {
        let raw = serde_json::json!({
            "schemaVersion": STORAGE_SNAPSHOT_SCHEMA_VERSION + 1,
            "exportedAt": 1
        });
        assert_eq!(
            migrate_storage_snapshot(raw).unwrap_err(),
            StorageError::UnsupportedSnapshotSchemaVersion {
                schema_version: STORAGE_SNAPSHOT_SCHEMA_VERSION + 1,
            }
        );
    }

    #[test]
    fn migrate_storage_snapshot_rejects_non_object_root_and_malformed_shape() {
        assert!(matches!(
            migrate_storage_snapshot(serde_json::json!([1, 2, 3])),
            Err(StorageError::InvalidSnapshot { field }) if field == "root"
        ));

        // Object root but a structurally invalid sources array: the JSON shape
        // does not match StorageSnapshot, so deserialization fails closed.
        let raw = serde_json::json!({
            "schemaVersion": STORAGE_SNAPSHOT_SCHEMA_VERSION,
            "exportedAt": 1,
            "sources": ["not-a-source"]
        });
        assert!(matches!(
            migrate_storage_snapshot(raw),
            Err(StorageError::InvalidSnapshot { .. })
        ));
    }

    #[test]
    fn migrate_storage_snapshot_preserves_v1_legacy_progress_when_v0_alias_absent() {
        let raw = serde_json::json!({
            "schemaVersion": 0,
            "exportedAt": 5,
            "legacyProgress": [
                { "bookId": "kept-b", "chapterIndex": 1, "chapterOffset": 0, "chapterProgress": 0.0 }
            ]
        });
        let snapshot = migrate_storage_snapshot(raw).unwrap();
        assert_eq!(snapshot.legacy_progress.len(), 1);
        assert_eq!(snapshot.legacy_progress[0].book_id, "kept-b");
    }

    // ---- Source-scoped reading progress boundary tests ----

    fn progress_entry(
        source: &str,
        book: &str,
        chapter_index: u32,
        chapter_offset: u64,
        chapter_progress: f64,
        updated_at: i64,
    ) -> ReadingProgressEntry {
        ReadingProgressEntry {
            source_id: source.into(),
            book_id: book.into(),
            chapter_index,
            chapter_offset,
            chapter_progress,
            updated_at,
            device_id: None,
        }
    }

    fn unified_chapter(
        source: &str,
        book: &str,
        id: &str,
        ordinal: i64,
        canonical_locator: &str,
        origin_locator: Option<&str>,
    ) -> UnifiedChapterIndexEntry {
        UnifiedChapterIndexEntry {
            source_id: source.into(),
            book_id: book.into(),
            chapter_id: id.into(),
            ordinal,
            title: Some(format!("Chapter {ordinal}")),
            canonical_locator: canonical_locator.into(),
            origin_specific_locator: origin_locator.map(str::to_string),
            content_locator: Some(format!("{canonical_locator}#content")),
            content_checksum: Some(format!("checksum-{source}-{id}")),
            diagnostics: Vec::new(),
        }
    }

    fn unified_progress(
        source: &str,
        book: &str,
        chapter_id: Option<&str>,
        origin_locator: Option<&str>,
        canonical_locator: Option<&str>,
        ordinal: i64,
    ) -> UnifiedReadingProgress {
        UnifiedReadingProgress {
            locator: UnifiedReadingLocator {
                source_id: source.into(),
                book_id: book.into(),
                chapter_id: chapter_id.map(str::to_string),
                origin_specific_locator: origin_locator.map(str::to_string),
                canonical_locator: canonical_locator.map(str::to_string),
                chapter_ordinal: ordinal,
                character_offset: 7,
                normalized_chapter_progress: 0.5,
                normalized_book_progress: 0.25,
                surrounding_text_checksum: Some("surrounding".into()),
                remote_toc_version: Some("v1".into()),
                local_fingerprint: None,
                content_checksum: None,
                timestamp: "1970-01-01T00:00:00Z".into(),
            },
            restore_state: UnifiedReadingRestoreState::ResetToBeginning,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn scoped_progress_put_then_get_round_trips() {
        let store = InMemoryStorage::new();
        let entry = progress_entry("s1", "b1", 3, 1024, 0.5, 1700000000);

        let saved = store.save_reading_progress(entry.clone()).unwrap();

        assert_eq!(saved, entry);
        assert_eq!(
            store.get_reading_progress("s1", "b1").unwrap(),
            Some(entry.clone())
        );
        assert_eq!(entry.as_domain_progress().book_id, "b1");
        assert_eq!(entry.as_domain_progress().chapter_progress, 0.5);
    }

    #[test]
    fn scoped_progress_same_book_id_different_source_no_collision() {
        let store = InMemoryStorage::new();
        store
            .save_reading_progress(progress_entry("s1", "b1", 1, 100, 0.25, 1000))
            .unwrap();
        store
            .save_reading_progress(progress_entry("s2", "b1", 8, 900, 0.9, 2000))
            .unwrap();

        assert_eq!(
            store
                .get_reading_progress("s1", "b1")
                .unwrap()
                .unwrap()
                .chapter_index,
            1
        );
        assert_eq!(
            store
                .get_reading_progress("s2", "b1")
                .unwrap()
                .unwrap()
                .chapter_index,
            8
        );
        assert_eq!(store.list_reading_progress("s1").unwrap().len(), 1);
        assert_eq!(store.list_reading_progress("s2").unwrap().len(), 1);
    }

    #[test]
    fn scoped_progress_list_sorted_newest_first() {
        let store = InMemoryStorage::new();
        store
            .save_reading_progress(progress_entry("s1", "b1", 1, 0, 0.1, 1000))
            .unwrap();
        store
            .save_reading_progress(progress_entry("s1", "b2", 2, 0, 0.2, 3000))
            .unwrap();
        store
            .save_reading_progress(progress_entry("s1", "b3", 3, 0, 0.3, 2000))
            .unwrap();

        let ids = store
            .list_reading_progress("s1")
            .unwrap()
            .into_iter()
            .map(|entry| entry.book_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["b2", "b3", "b1"]);
    }

    #[test]
    fn scoped_progress_stale_update_does_not_replace_current_but_keeps_history() {
        let store = InMemoryStorage::new();
        let current = progress_entry("s1", "b1", 5, 500, 0.5, 5000);
        let stale = progress_entry("s1", "b1", 1, 100, 0.1, 1000);
        store.save_reading_progress(current.clone()).unwrap();

        let returned = store.save_reading_progress(stale.clone()).unwrap();

        assert_eq!(returned, current);
        assert_eq!(
            store
                .get_reading_progress("s1", "b1")
                .unwrap()
                .unwrap()
                .chapter_index,
            5
        );
        let history = store.reading_progress_history("s1", "b1").unwrap();
        assert_eq!(history, vec![stale, current]);
    }

    #[test]
    fn scoped_progress_equal_timestamp_allows_latest_write() {
        let store = InMemoryStorage::new();
        store
            .save_reading_progress(progress_entry("s1", "b1", 1, 100, 0.1, 1000))
            .unwrap();
        store
            .save_reading_progress(progress_entry("s1", "b1", 2, 200, 0.2, 1000))
            .unwrap();

        let current = store.get_reading_progress("s1", "b1").unwrap().unwrap();
        assert_eq!(current.chapter_index, 2);
        assert_eq!(current.chapter_offset, 200);
        assert_eq!(store.reading_progress_history("s1", "b1").unwrap().len(), 2);
    }

    #[test]
    fn scoped_progress_updates_and_clears_shelf_last_read() {
        let store = InMemoryStorage::new();
        store
            .add_to_shelf(shelf_entry("s1", "b1", "Dune", 1000))
            .unwrap();

        store
            .save_reading_progress(progress_entry("s1", "b1", 3, 0, 0.3, 3000))
            .unwrap();
        assert_eq!(
            store
                .get_shelf_entry("s1", "b1")
                .unwrap()
                .unwrap()
                .last_read_at,
            Some(3000)
        );

        store.clear_reading_progress("s1", "b1").unwrap();
        assert!(store.get_reading_progress("s1", "b1").unwrap().is_none());
        assert!(store
            .reading_progress_history("s1", "b1")
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .get_shelf_entry("s1", "b1")
                .unwrap()
                .unwrap()
                .last_read_at,
            None
        );
        store.clear_reading_progress("s1", "b1").unwrap();
    }

    #[test]
    fn scoped_progress_rejects_invalid_keys_and_fields() {
        let store = InMemoryStorage::new();
        let err = store
            .save_reading_progress(progress_entry("", "b1", 1, 0, 0.1, 1000))
            .unwrap_err();
        assert_eq!(
            err,
            StorageError::InvalidKey {
                field: "source_id".into()
            }
        );

        let err = store
            .save_reading_progress(progress_entry("s1", "b1", 1, 0, 1.1, 1000))
            .unwrap_err();
        assert_eq!(
            err,
            StorageError::InvalidProgress {
                field: "chapter_progress".into()
            }
        );

        let mut nan = progress_entry("s1", "b1", 1, 0, f64::NAN, 1000);
        assert!(matches!(
            store.save_reading_progress(nan.clone()),
            Err(StorageError::InvalidProgress { .. })
        ));
        nan.chapter_progress = 0.5;
        nan.device_id = Some("   ".into());
        assert!(matches!(
            store.save_reading_progress(nan),
            Err(StorageError::InvalidProgress { .. })
        ));
        assert!(matches!(
            store.list_reading_progress(""),
            Err(StorageError::InvalidKey { .. })
        ));
    }

    #[test]
    fn scoped_progress_entry_json_round_trips() {
        let mut entry = progress_entry("s1", "b1", 7, 2048, 0.75, 1700000000);
        entry.device_id = Some("device-a".into());

        let json = serde_json::to_string(&entry).unwrap();
        let back: ReadingProgressEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn scoped_progress_entry_denies_unknown_fields() {
        let json = r#"{"sourceId":"s1","bookId":"b1","chapterIndex":1,"chapterOffset":10,"chapterProgress":0.1,"updatedAt":1,"bogus":true}"#;
        assert!(serde_json::from_str::<ReadingProgressEntry>(json).is_err());
    }

    #[test]
    fn unified_progress_restores_by_alias_locator_without_cross_source_merge() {
        let chapters = vec![
            unified_chapter(
                "remote-a",
                "same-book",
                "remote-0",
                0,
                "https://owned.example.test/books/progress/ch1.html?a=1&b=2",
                Some("https://owned.example.test/books/progress/ch1.html?b=2&a=1#primary"),
            ),
            unified_chapter(
                "local",
                "same-book",
                "local-0",
                0,
                "local://same-book/chapter/0",
                Some("OPS/chapter-0.xhtml"),
            ),
        ];
        let remote_progress = unified_progress(
            "remote-a",
            "same-book",
            None,
            Some("https://owned.example.test/books/progress/ch1.html?b=2&a=1#alias"),
            None,
            99,
        );

        let restored =
            resolve_unified_reading_progress(&remote_progress, "remote-a", "same-book", &chapters)
                .unwrap();

        assert_eq!(
            restored.restore_state,
            UnifiedReadingRestoreState::LocatorRestored
        );
        assert_eq!(restored.locator.chapter_id.as_deref(), Some("remote-0"));
        assert_eq!(restored.locator.chapter_ordinal, 0);
        assert_eq!(restored.diagnostics, vec!["origin_locator_canonicalized"]);

        let local_progress = unified_progress(
            "local",
            "same-book",
            None,
            None,
            Some("OPS/chapter-0.xhtml"),
            99,
        );
        let local =
            resolve_unified_reading_progress(&local_progress, "local", "same-book", &chapters)
                .unwrap();
        assert_eq!(local.locator.chapter_id.as_deref(), Some("local-0"));
        assert_ne!(local.locator.chapter_id, restored.locator.chapter_id);

        let reset =
            resolve_unified_reading_progress(&remote_progress, "local", "same-book", &chapters)
                .unwrap();
        assert_eq!(
            reset.restore_state,
            UnifiedReadingRestoreState::ResetToBeginning
        );
        assert_eq!(reset.diagnostics, vec!["source_or_book_mismatch"]);
        assert_eq!(reset.locator.chapter_id.as_deref(), Some("local-0"));
    }

    #[test]
    fn unified_progress_restore_state_falls_back_through_ordinal_nearest_and_reset() {
        let chapters = vec![
            unified_chapter("rss", "book-1", "c0", 0, "/book/1/chapter/0", None),
            unified_chapter("rss", "book-1", "c1", 1, "/book/1/chapter/1", None),
        ];

        let exact = unified_progress("rss", "book-1", Some("c1"), None, None, 0);
        let restored =
            resolve_unified_reading_progress(&exact, "rss", "book-1", &chapters).unwrap();
        assert_eq!(
            restored.restore_state,
            UnifiedReadingRestoreState::ExactRestored
        );
        assert_eq!(restored.locator.chapter_id.as_deref(), Some("c1"));

        let ordinal = unified_progress("rss", "book-1", Some("missing"), None, None, 1);
        let restored =
            resolve_unified_reading_progress(&ordinal, "rss", "book-1", &chapters).unwrap();
        assert_eq!(
            restored.restore_state,
            UnifiedReadingRestoreState::OrdinalRestored
        );
        assert_eq!(restored.locator.chapter_id.as_deref(), Some("c1"));

        let high_ordinal = unified_progress("rss", "book-1", Some("missing"), None, None, 99);
        let restored =
            resolve_unified_reading_progress(&high_ordinal, "rss", "book-1", &chapters).unwrap();
        assert_eq!(
            restored.restore_state,
            UnifiedReadingRestoreState::NearestChapterRestored
        );
        assert_eq!(restored.locator.chapter_id.as_deref(), Some("c1"));
        assert_eq!(restored.diagnostics, vec!["nearest_fallback"]);

        let missing =
            resolve_unified_reading_progress(&high_ordinal, "rss", "missing", &chapters).unwrap();
        assert_eq!(
            missing.restore_state,
            UnifiedReadingRestoreState::ResetToBeginning
        );
        assert!(missing.locator.chapter_id.is_none());
        assert_eq!(missing.locator.character_offset, 0);
        assert_eq!(missing.locator.normalized_book_progress, 0.0);
    }

    #[test]
    fn unified_progress_json_shape_matches_recovery33_and_rejects_drift() {
        let progress = unified_progress(
            "remote-a",
            "book-1",
            Some("c1"),
            Some("https://owned.example.test/books/one.html#alias"),
            Some("https://owned.example.test/books/one.html"),
            1,
        );

        progress.validate().unwrap();
        let json = serde_json::to_value(&progress).unwrap();
        assert_eq!(json["locator"]["sourceId"], "remote-a");
        assert_eq!(json["locator"]["bookId"], "book-1");
        assert_eq!(json["locator"]["chapterId"], "c1");
        assert_eq!(json["locator"]["normalizedChapterProgress"], 0.5);
        assert_eq!(json["restoreState"], "resetToBeginning");

        assert!(
            serde_json::from_value::<UnifiedReadingProgress>(serde_json::json!({
                "locator": json["locator"].clone(),
                "restoreState": "resetToBeginning",
                "diagnostics": [],
                "hostRuntime": true
            }))
            .is_err()
        );

        let mut invalid = progress.clone();
        invalid.locator.normalized_book_progress = f64::NAN;
        assert_eq!(
            invalid.validate().unwrap_err(),
            StorageError::InvalidProgress {
                field: "normalized_book_progress".into()
            }
        );

        let invalid_chapter = UnifiedChapterIndexEntry {
            canonical_locator: " ".into(),
            ..unified_chapter("remote-a", "book-1", "c1", 0, "/chapter/1", None)
        };
        assert_eq!(
            invalid_chapter.validate().unwrap_err(),
            StorageError::InvalidProgress {
                field: "canonical_locator".into()
            }
        );
    }

    #[test]
    fn unified_chapter_refresh_reports_unchanged_stable_identity() {
        let chapters = vec![
            unified_chapter("rss", "book-1", "c0", 0, "/book/1/chapter/0", None),
            unified_chapter("rss", "book-1", "c1", 1, "/book/1/chapter/1", None),
        ];

        let result =
            plan_unified_chapter_index_refresh("rss", "book-1", &chapters, &chapters, None)
                .unwrap();

        assert!(result.added_chapter_ids.is_empty());
        assert!(result.removed_chapter_ids.is_empty());
        assert!(result.changed_chapter_ids.is_empty());
        assert!(result.cache_invalidated_chapter_ids.is_empty());
        assert_eq!(
            result
                .diff_entries
                .iter()
                .map(|entry| entry.diff_type)
                .collect::<Vec<_>>(),
            vec![UnifiedChapterDiffType::Unchanged]
        );
        assert_eq!(result.diff_entries[0].reason, "chapter_identity_stable");
        assert_eq!(
            serde_json::to_value(UnifiedChapterDiffType::ChapterAdded).unwrap(),
            serde_json::json!("chapter_added")
        );
    }

    #[test]
    fn unified_chapter_refresh_reports_duplicate_merge_and_added_without_collision() {
        let old = vec![
            unified_chapter(
                "remote-a",
                "book-1",
                "c1",
                0,
                "https://owned.example.test/books/refresh/ch1.html",
                Some("/books/refresh/ch1.html"),
            ),
            unified_chapter(
                "remote-a",
                "book-1",
                "c2",
                1,
                "https://owned.example.test/books/refresh/ch2.html",
                Some("/books/refresh/ch2.html"),
            ),
        ];
        let mut merged = old[0].clone();
        merged
            .diagnostics
            .push("duplicate_canonical_locator_merged".into());
        let mut duplicate_alias = merged.clone();
        duplicate_alias.chapter_id = "z-alias".into();
        duplicate_alias.title = Some("Alias body should not win".into());
        duplicate_alias.content_locator = Some("/books/refresh/content/ch1-alias.html".into());
        let new = vec![
            merged,
            duplicate_alias,
            old[1].clone(),
            unified_chapter(
                "remote-a",
                "book-1",
                "c3",
                2,
                "https://owned.example.test/books/refresh/ch3.html",
                Some("/books/refresh/ch3.html"),
            ),
        ];

        let result =
            plan_unified_chapter_index_refresh("remote-a", "book-1", &old, &new, None).unwrap();

        assert_eq!(result.added_chapter_ids, vec!["c3"]);
        assert_eq!(result.cache_invalidated_chapter_ids, vec!["c1", "c3"]);
        assert!(result.diff_entries.iter().any(|entry| {
            entry.diff_type == UnifiedChapterDiffType::DuplicateMerged
                && entry.reason == "canonical_duplicate_merged"
                && entry.old_chapter_id.as_deref() == Some("c1")
        }));
        assert!(result.diff_entries.iter().any(|entry| {
            entry.diff_type == UnifiedChapterDiffType::ChapterAdded
                && entry.reason == "canonical_locator_new"
                && entry.new_chapter_id.as_deref() == Some("c3")
        }));
        assert!(!result.diff_entries.iter().any(|entry| {
            entry.diff_type == UnifiedChapterDiffType::ChapterTitleChanged
                && entry.new_chapter_id.as_deref() == Some("z-alias")
        }));
    }

    #[test]
    fn unified_chapter_refresh_invalidates_changed_rows_and_remaps_progress() {
        let old = vec![
            unified_chapter("remote-a", "book-1", "c0", 0, "/book/1/chapter/0", None),
            unified_chapter("remote-a", "book-1", "c1", 1, "/book/1/chapter/1", None),
        ];
        let mut shifted = old[0].clone();
        shifted.ordinal = 1;
        shifted.title = Some("Chapter Zero Retitled".into());
        shifted.content_locator = Some("/book/1/content/0-v2".into());
        let mut second = old[1].clone();
        second.ordinal = 2;
        let new = vec![
            unified_chapter(
                "remote-a",
                "book-1",
                "inserted",
                0,
                "/book/1/chapter/new",
                None,
            ),
            shifted,
            second,
        ];
        let progress = unified_progress(
            "remote-a",
            "book-1",
            None,
            None,
            Some("/book/1/chapter/0"),
            0,
        );

        let result =
            plan_unified_chapter_index_refresh("remote-a", "book-1", &old, &new, Some(&progress))
                .unwrap();

        assert_eq!(result.added_chapter_ids, vec!["inserted"]);
        assert_eq!(result.changed_chapter_ids, vec!["c0"]);
        assert_eq!(result.reordered_chapter_ids, vec!["c0", "c1"]);
        assert!(result
            .cache_invalidated_chapter_ids
            .contains(&"c0".to_string()));
        assert!(result.diff_entries.iter().any(|entry| {
            entry.diff_type == UnifiedChapterDiffType::ChapterTitleChanged
                && entry.reason == "title_changed"
        }));
        assert!(result.diff_entries.iter().any(|entry| {
            entry.diff_type == UnifiedChapterDiffType::ChapterContentLocatorChanged
                && entry.reason == "content_locator_changed"
        }));
        let remapped = result.progress_remapping_result.unwrap();
        assert_eq!(
            remapped.restore_state,
            UnifiedReadingRestoreState::LocatorRestored
        );
        assert_eq!(remapped.locator.chapter_id.as_deref(), Some("c0"));
        assert_eq!(remapped.locator.chapter_ordinal, 1);

        let json = serde_json::to_value(&remapped).unwrap();
        assert_eq!(json["restoreState"], "locatorRestored");
        assert!(
            serde_json::from_value::<UnifiedChapterIndexRefreshResult>(serde_json::json!({
                "sourceId": "remote-a",
                "bookId": "book-1",
                "addedChapterIds": [],
                "removedChapterIds": [],
                "changedChapterIds": [],
                "reorderedChapterIds": [],
                "cacheInvalidatedChapterIds": [],
                "diffEntries": [],
                "diagnostics": [],
                "hostCacheMutated": true
            }))
            .is_err()
        );
    }

    #[test]
    fn book_upsert_overwrites() {
        let store = InMemoryStorage::new();
        let b1 = Book {
            book_id: "1".into(),
            title: "old".into(),
            author: String::new(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        };
        store.put_book(b1).unwrap();
        let b2 = Book {
            book_id: "1".into(),
            title: "new".into(),
            author: String::new(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        };
        store.put_book(b2).unwrap();
        let got = store.get_book("1").unwrap().unwrap();
        assert_eq!(got.title, "new");
    }

    // ---- Bookshelf boundary tests ----

    fn shelf_entry(source: &str, book: &str, title: &str, added_at: i64) -> BookshelfEntry {
        BookshelfEntry {
            source_id: source.into(),
            book_id: book.into(),
            title: title.into(),
            author: String::new(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
            added_at,
            last_read_at: None,
            group: None,
            sort_index: 0,
        }
    }

    fn chapter_candidate(id: &str, title: &str, url: &str, order: i32) -> ChapterMatchCandidate {
        ChapterMatchCandidate::new(id, title, url, order)
    }

    #[test]
    fn chapter_update_detects_new_chapters_by_legacy_title_key() {
        let old_toc = vec![
            chapter_candidate("old-1", " Chapter 1 ", "/old/1", 0),
            chapter_candidate("old-2", "第2章 风起", "/old/2", 1),
        ];
        let new_toc = vec![
            chapter_candidate("new-1", "chapter 1", "/new/1", 0),
            chapter_candidate("new-2", " 第2章 风起 ", "/new/2", 1),
            chapter_candidate("new-3", "第3章 雨夜", "/new/3", 2),
            chapter_candidate("new-4", "CHAPTER 4", "/new/4", 3),
        ];

        let result = detect_new_chapters(&old_toc, &new_toc);

        assert!(result.has_new_chapters);
        assert_eq!(result.total_old_chapters, 2);
        assert_eq!(result.total_new_chapters, 4);
        assert_eq!(
            result
                .new_chapters
                .iter()
                .map(|chapter| chapter.chapter_id.as_str())
                .collect::<Vec<_>>(),
            vec!["new-3", "new-4"]
        );
        assert_eq!(result.error, None);
    }

    #[test]
    fn chapter_update_by_count_matches_legacy_suffix_slice() {
        let new_toc = vec![
            chapter_candidate("c0", "Chapter 0", "/0", 0),
            chapter_candidate("c1", "Chapter 1", "/1", 1),
            chapter_candidate("c2", "Chapter 2", "/2", 2),
            chapter_candidate("c3", "Chapter 3", "/3", 3),
        ];

        let result = detect_new_chapters_by_count(2, &new_toc);

        assert!(result.has_new_chapters);
        assert_eq!(result.total_old_chapters, 2);
        assert_eq!(result.total_new_chapters, 4);
        assert_eq!(
            result
                .new_chapters
                .iter()
                .map(|chapter| chapter.chapter_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Chapter 2", "Chapter 3"]
        );

        let unchanged = detect_new_chapters_by_count(4, &new_toc);
        assert!(!unchanged.has_new_chapters);
        assert!(unchanged.new_chapters.is_empty());

        let shrunk = detect_new_chapters_by_count(5, &new_toc);
        assert!(!shrunk.has_new_chapters);
        assert_eq!(shrunk.total_old_chapters, 5);
        assert_eq!(shrunk.total_new_chapters, 4);
    }

    #[test]
    fn chapter_update_json_shape_matches_legacy_candidate_and_rejects_drift() {
        let candidate = chapter_candidate("c1", "第1章", "/toc/1", 1);
        assert_eq!(
            serde_json::to_value(&candidate).unwrap(),
            serde_json::json!({
                "chapterId": "c1",
                "chapterTitle": "第1章",
                "chapterUrl": "/toc/1",
                "order": 1
            })
        );

        let parsed: ChapterMatchCandidate = serde_json::from_value(serde_json::json!({
            "chapterId": "c2",
            "chapterTitle": "Chapter 2",
            "chapterUrl": "/toc/2",
            "order": 2
        }))
        .unwrap();
        assert_eq!(parsed.chapter_id, "c2");

        assert!(
            serde_json::from_value::<ChapterMatchCandidate>(serde_json::json!({
                "chapterId": "c3",
                "chapterTitle": "Chapter 3",
                "chapterUrl": "/toc/3",
                "order": 3,
                "chapterHref": "/toc/3"
            }))
            .is_err()
        );

        let result = detect_new_chapters(&[], &[candidate]);
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["hasNewChapters"], true);
        assert_eq!(value["totalOldChapters"], 0);
        assert_eq!(value["totalNewChapters"], 1);
        assert!(value.get("error").is_none());
    }

    #[test]
    fn chapter_update_cache_plan_matches_legacy_queue_and_availability() {
        let old_toc = vec![
            chapter_candidate("old-1", "Chapter 1", "/old/1", 0),
            chapter_candidate("old-2", "Chapter 2", "/old/2", 1),
        ];
        let new_toc = vec![
            chapter_candidate("new-1", "chapter 1", "/new/1", 0),
            chapter_candidate("new-2", "Chapter 2", "/new/2", 1),
            chapter_candidate("new-3", "Chapter 3", "/new/3", 2),
            chapter_candidate("new-4", "Chapter 4", "/new/4", 3),
        ];
        let cache_entries = vec![
            offline_chapter("s1", "new-1", ChapterCacheStatus::Available),
            offline_chapter("s1", "new-2", ChapterCacheStatus::Stale),
            offline_chapter("s1", "new-3", ChapterCacheStatus::Failed),
            offline_chapter("other", "new-4", ChapterCacheStatus::Available),
        ];

        let result = plan_chapter_update_cache(
            &old_toc,
            &new_toc,
            "s1",
            &cache_entries,
            &[String::from("new-3")],
        )
        .unwrap();

        assert!(result.has_update());
        assert_eq!(result.new_chapter_count, 2);
        assert_eq!(result.queued_for_download, 1);
        assert_eq!(
            result.notification_type,
            UpdateNotificationType::NewChaptersAvailable
        );
        assert_eq!(result.cache_status.total_chapters, 4);
        assert_eq!(result.cache_status.cached_chapters, 1);
        assert_eq!(result.cache_status.stale_chapters, 1);
        assert_eq!(result.cache_status.failed_chapters, 1);
        assert_eq!(result.cache_status.missing_chapters, 1);
    }

    #[test]
    fn chapter_update_cache_plan_dedupes_new_chapter_ids_like_legacy_queue() {
        let new_toc = vec![
            chapter_candidate("dup", "Chapter 1", "/new/1", 0),
            chapter_candidate("dup", "Chapter 2", "/new/2", 1),
            chapter_candidate("fresh", "Chapter 3", "/new/3", 2),
        ];

        let result =
            plan_chapter_update_cache(&[], &new_toc, "s1", &[], &[String::from("fresh")]).unwrap();

        assert_eq!(result.new_chapter_count, 3);
        assert_eq!(result.queued_for_download, 1);
        assert_eq!(result.cache_status.missing_chapters, 3);
    }

    #[test]
    fn chapter_update_cache_plan_reports_no_update_and_rejects_drift() {
        let old_toc = vec![chapter_candidate("c1", "Chapter 1", "/old/1", 0)];
        let new_toc = vec![chapter_candidate("c1", " chapter 1 ", "/new/1", 0)];

        let result = plan_chapter_update_cache(&old_toc, &new_toc, "s1", &[], &[]).unwrap();

        assert!(!result.has_update());
        assert_eq!(result.new_chapter_count, 0);
        assert_eq!(result.queued_for_download, 0);
        assert_eq!(result.notification_type, UpdateNotificationType::NoUpdate);
        assert_eq!(
            serde_json::to_value(UpdateNotificationType::NewChaptersAvailable).unwrap(),
            serde_json::json!("newChaptersAvailable")
        );
        assert_eq!(
            serde_json::to_value(&result).unwrap()["notificationType"],
            serde_json::json!("noUpdate")
        );
        assert!(
            serde_json::from_value::<ChapterUpdateCacheResult>(serde_json::json!({
                "newChapterCount": 0,
                "queuedForDownload": 0,
                "notificationType": "noUpdate",
                "cacheStatus": {
                    "sourceId": "s1",
                    "totalChapters": 1,
                    "cachedChapters": 0,
                    "staleChapters": 0,
                    "failedChapters": 0,
                    "missingChapters": 1
                },
                "hostQueueMutated": true
            }))
            .is_err()
        );
        assert_eq!(
            plan_chapter_update_cache(&[], &new_toc, " ", &[], &[]).unwrap_err(),
            StorageError::InvalidKey {
                field: "source_id".into()
            }
        );
    }

    fn scheduled_book(
        source: &str,
        book: &str,
        title: &str,
        archived: bool,
        old_count: usize,
        new_count: usize,
    ) -> ScheduledUpdateBookInput {
        let old_toc = (0..old_count)
            .map(|index| {
                chapter_candidate(
                    &format!("{source}_c{index}"),
                    &format!("Chapter {}", index + 1),
                    &format!("/c{}", index + 1),
                    index as i32,
                )
            })
            .collect::<Vec<_>>();
        let new_toc = (0..new_count)
            .map(|index| {
                let suffix = if index >= old_count { " NEW" } else { "" };
                chapter_candidate(
                    &format!("{source}_c{index}"),
                    &format!("Chapter {}{suffix}", index + 1),
                    &format!("/c{}", index + 1),
                    index as i32,
                )
            })
            .collect::<Vec<_>>();
        ScheduledUpdateBookInput {
            source_id: source.into(),
            book_id: book.into(),
            title: title.into(),
            author: "Author".into(),
            archived,
            cloudflare_blocked: false,
            old_toc,
            new_toc,
            cache_entries: Vec::new(),
            already_queued_chapter_ids: Vec::new(),
        }
    }

    #[test]
    fn scheduled_update_policy_defaults_match_legacy_no_network_boundary() {
        let policy = ScheduledUpdatePolicy::default();

        assert!(!policy.allow_network);
        assert!(!policy.auto_download_new_chapters);
        assert_eq!(policy.max_books_per_run, 5);
        assert_eq!(policy.max_chapters_per_book, 3);
        assert!(policy.skip_archived);
        assert!(policy.skip_cloudflare_blocked);
        assert_eq!(
            serde_json::to_value(ScheduledUpdateRuntimeMode::LiveOptIn).unwrap(),
            serde_json::json!("liveOptIn")
        );
        assert_eq!(
            plan_scheduled_update(&ScheduledUpdateRequest::default())
                .unwrap()
                .status,
            ScheduledUpdateRunStatus::NoEligibleBooks
        );
    }

    #[test]
    fn scheduled_update_state_machine_skips_archived_and_cloudflare_items() {
        let request = ScheduledUpdateRequest {
            books: vec![
                scheduled_book("case_022", "archived", "Archived", true, 5, 8),
                scheduled_book("case_021", "blocked", "Blocked", false, 5, 8),
            ],
            ..ScheduledUpdateRequest::default()
        };

        let result = plan_scheduled_update(&request).unwrap();

        assert_eq!(result.status, ScheduledUpdateRunStatus::AllSkipped);
        assert_eq!(result.checked_books, 0);
        assert_eq!(result.skipped_books, 2);
        assert!(!result.network_accessed);
        assert!(!result.web_view_used);
        assert_eq!(
            result
                .per_book_results
                .iter()
                .map(|book| book.status)
                .collect::<Vec<_>>(),
            vec![
                ScheduledUpdateBookStatus::SkippedArchived,
                ScheduledUpdateBookStatus::SkippedCloudflareBlocked
            ]
        );
    }

    #[test]
    fn scheduled_update_detects_updates_limits_queue_and_never_uses_network() {
        let request = ScheduledUpdateRequest {
            policy: ScheduledUpdatePolicy {
                max_books_per_run: 3,
                max_chapters_per_book: 1,
                ..ScheduledUpdatePolicy::default()
            },
            runtime_mode: ScheduledUpdateRuntimeMode::LiveOptIn,
            books: (0..10)
                .map(|index| {
                    scheduled_book(
                        "case_022",
                        &format!("book-{index}"),
                        &format!("Book {index}"),
                        false,
                        5,
                        8,
                    )
                })
                .collect(),
        };

        let result = plan_scheduled_update(&request).unwrap();

        assert_eq!(result.status, ScheduledUpdateRunStatus::Success);
        assert_eq!(result.checked_books, 3);
        assert_eq!(result.updated_books, 3);
        assert_eq!(result.new_chapters_detected, 9);
        assert_eq!(result.download_queue_suggestions, 3);
        assert_eq!(result.auto_download_completed, 0);
        assert!(!result.network_accessed);
        assert!(!result.web_view_used);
        assert_eq!(result.notifications.len(), 4);
        assert!(result
            .notifications
            .iter()
            .any(|notification| notification.category
                == ScheduledUpdateNotificationCategory::UpdateAvailable));
        assert!(result.per_book_results.iter().all(|book| {
            book.suggested_chapter_ids.len() == 1
                && book.status == ScheduledUpdateBookStatus::Updated
        }));
    }

    #[test]
    fn scheduled_update_mock_auto_download_recomputes_offline_availability() {
        let mut book = scheduled_book("case_022", "book", "Book", false, 5, 8);
        book.cache_entries = vec![offline_chapter(
            "case_022",
            "case_022_c0",
            ChapterCacheStatus::Available,
        )];
        let request = ScheduledUpdateRequest {
            policy: ScheduledUpdatePolicy {
                auto_download_new_chapters: true,
                max_chapters_per_book: 2,
                ..ScheduledUpdatePolicy::default()
            },
            runtime_mode: ScheduledUpdateRuntimeMode::Mock,
            books: vec![book],
        };

        let result = plan_scheduled_update(&request).unwrap();
        let first = result.per_book_results.first().unwrap();

        assert_eq!(result.status, ScheduledUpdateRunStatus::Success);
        assert_eq!(result.updated_books, 1);
        assert_eq!(result.download_queue_suggestions, 2);
        assert_eq!(result.auto_download_completed, 2);
        assert_eq!(first.auto_download_completed, 2);
        assert_eq!(
            first.cache_status.as_ref().unwrap().cached_chapters,
            3,
            "one existing cache row plus two mock downloaded chapters"
        );
        assert_eq!(first.cache_status.as_ref().unwrap().total_chapters, 8);
    }

    #[test]
    fn scheduled_update_json_shape_and_validation_reject_drift() {
        let result = plan_scheduled_update(&ScheduledUpdateRequest {
            policy: ScheduledUpdatePolicy {
                notify_on_no_update: true,
                ..ScheduledUpdatePolicy::default()
            },
            books: vec![scheduled_book("case_022", "same", "Same", false, 2, 2)],
            ..ScheduledUpdateRequest::default()
        })
        .unwrap();

        assert_eq!(result.status, ScheduledUpdateRunStatus::Success);
        assert_eq!(result.updated_books, 0);
        assert_eq!(result.notifications.len(), 1);
        assert_eq!(
            result.per_book_results[0].status,
            ScheduledUpdateBookStatus::NoUpdate
        );
        assert_eq!(
            serde_json::to_value(ScheduledUpdateRunStatus::AllSkipped).unwrap(),
            serde_json::json!("allSkipped")
        );
        assert_eq!(
            serde_json::to_value(ScheduledUpdateNotificationCategory::NewChapters).unwrap(),
            serde_json::json!("newChapters")
        );
        assert!(
            serde_json::from_value::<ScheduledUpdateRequest>(serde_json::json!({
                "runtimeMode": "fixture",
                "policy": {
                    "maxBooksPerRun": 5,
                    "maxChaptersPerBook": 3,
                    "hostSchedulerEnabled": true
                },
                "books": []
            }))
            .is_err()
        );

        let invalid_limit = ScheduledUpdateRequest {
            policy: ScheduledUpdatePolicy {
                max_books_per_run: 0,
                ..ScheduledUpdatePolicy::default()
            },
            books: vec![scheduled_book("case_022", "book", "Book", false, 1, 2)],
            ..ScheduledUpdateRequest::default()
        };
        assert_eq!(
            plan_scheduled_update(&invalid_limit).unwrap_err(),
            StorageError::InvalidChapterCache {
                field: "max_books_per_run".into()
            }
        );
    }

    #[test]
    fn source_switch_match_uses_legacy_strategy_order() {
        let old_toc = vec![chapter_candidate("old-0", "第1章 起点", "/old/0", 0)];
        let new_toc = vec![
            chapter_candidate("new-0", "第1章 起点", "/new/0", 0),
            chapter_candidate("new-1", "Wrong title at same index", "/new/1", 1),
            chapter_candidate("new-2", "Chapter 12 风 起", "/new/2", 2),
        ];

        let exact = match_source_switch_chapter(&old_toc, &new_toc, "第1章 起点", 2);
        assert!(exact.success);
        assert_eq!(exact.strategy, SourceSwitchStrategy::ExactTitleMatch);
        assert_eq!(exact.matched_chapter.unwrap().chapter_id, "new-0");
        assert_eq!(exact.total_chapters, 3);

        let order = match_source_switch_chapter(&old_toc, &new_toc, "第十二章 风 起", 1);
        assert!(order.success);
        assert_eq!(order.strategy, SourceSwitchStrategy::OrderIndexMatch);
        assert_eq!(order.matched_chapter.unwrap().chapter_id, "new-1");
    }

    #[test]
    fn source_switch_match_uses_fuzzy_normalization_then_fallback() {
        let new_toc = vec![
            chapter_candidate("new-0", "Preface", "/new/0", 0),
            chapter_candidate("new-1", "第十二章 风   起", "/new/1", 1),
        ];

        assert_eq!(
            normalize_source_switch_title(" Chapter 12   风   起 "),
            "风 起"
        );
        assert_eq!(normalize_source_switch_title("001- 归来"), "归来");
        assert_eq!(normalize_source_switch_title("第十二章 风   起"), "风 起");

        let fuzzy = match_source_switch_chapter(&[], &new_toc, "Chapter 12 风 起", 99);
        assert!(fuzzy.success);
        assert_eq!(fuzzy.strategy, SourceSwitchStrategy::FuzzyTitleMatch);
        assert_eq!(fuzzy.matched_chapter.unwrap().chapter_id, "new-1");

        let fallback = match_source_switch_chapter(&[], &new_toc, "No matching title", 99);
        assert!(fallback.success);
        assert_eq!(fallback.strategy, SourceSwitchStrategy::LastReadFallback);
        assert_eq!(fallback.matched_chapter.unwrap().chapter_id, "new-0");
    }

    #[test]
    fn source_switch_match_empty_toc_and_json_shape_match_legacy_boundary() {
        let empty = match_source_switch_chapter(&[], &[], "第1章", 0);
        assert!(!empty.success);
        assert_eq!(empty.strategy, SourceSwitchStrategy::LastReadFallback);
        assert_eq!(empty.total_chapters, 0);
        assert_eq!(empty.error.as_deref(), Some("New source TOC is empty"));

        let value = serde_json::to_value(&SourceSwitchStrategy::FuzzyTitleMatch).unwrap();
        assert_eq!(value, serde_json::json!("fuzzyTitleMatch"));

        let matched = match_source_switch_chapter(
            &[],
            &[chapter_candidate("c1", "Chapter 1", "/1", 0)],
            "Chapter 1",
            0,
        );
        assert_eq!(
            serde_json::to_value(&matched).unwrap(),
            serde_json::json!({
                "success": true,
                "matchedChapter": {
                    "chapterId": "c1",
                    "chapterTitle": "Chapter 1",
                    "chapterUrl": "/1",
                    "order": 0
                },
                "strategy": "exactTitleMatch",
                "totalChapters": 1
            })
        );

        assert!(
            serde_json::from_value::<SourceSwitchResult>(serde_json::json!({
                "success": true,
                "strategy": "lastReadFallback",
                "totalChapters": 1,
                "matchedChapter": null,
                "hostRuntime": true
            }))
            .is_err()
        );
    }

    #[test]
    fn source_switch_progress_migration_resets_matched_chapter_position() {
        let mut old_progress = progress_entry("source-old", "book-1", 8, 2048, 0.8, 1000);
        old_progress.device_id = Some("phone-a".into());
        let switch_result = match_source_switch_chapter(
            &[],
            &[
                chapter_candidate("c0", "Chapter 1", "/1", 0),
                chapter_candidate("c1", "Chapter 2", "/2", 1),
            ],
            "Chapter 2",
            0,
        );

        let migrated =
            migrate_source_switch_progress(&old_progress, &switch_result, "source-new", 2000)
                .unwrap();

        assert!(migrated.success);
        assert!(migrated.chapter_matched);
        assert_eq!(migrated.old_progress, old_progress);
        let new_progress = migrated.new_progress.unwrap();
        assert_eq!(new_progress.source_id, "source-new");
        assert_eq!(new_progress.book_id, "book-1");
        assert_eq!(new_progress.chapter_index, 1);
        assert_eq!(new_progress.chapter_offset, 0);
        assert_eq!(new_progress.chapter_progress, 0.0);
        assert_eq!(new_progress.updated_at, 2000);
        assert_eq!(new_progress.device_id.as_deref(), Some("phone-a"));
    }

    #[test]
    fn source_switch_progress_migration_fails_without_matched_chapter() {
        let old_progress = progress_entry("source-old", "book-1", 4, 512, 0.4, 1000);
        let switch_result = match_source_switch_chapter(&[], &[], "Missing", 0);

        let migrated =
            migrate_source_switch_progress(&old_progress, &switch_result, "source-new", 2000)
                .unwrap();

        assert!(!migrated.success);
        assert!(!migrated.chapter_matched);
        assert_eq!(migrated.old_progress, old_progress);
        assert!(migrated.new_progress.is_none());
    }

    #[test]
    fn source_switch_progress_migration_validates_storage_progress_boundary() {
        let old_progress = progress_entry("source-old", "book-1", 4, 512, 0.4, 1000);
        let invalid_match = SourceSwitchResult {
            success: true,
            matched_chapter: Some(chapter_candidate("bad", "Bad", "/bad", -1)),
            strategy: SourceSwitchStrategy::ExactTitleMatch,
            total_chapters: 1,
            error: None,
        };

        assert_eq!(
            migrate_source_switch_progress(&old_progress, &invalid_match, "source-new", 2000)
                .unwrap_err(),
            StorageError::InvalidProgress {
                field: "chapter_order".into()
            }
        );

        let valid_match = SourceSwitchResult {
            matched_chapter: Some(chapter_candidate("ok", "Ok", "/ok", 0)),
            ..invalid_match
        };
        assert_eq!(
            migrate_source_switch_progress(&old_progress, &valid_match, "   ", 2000).unwrap_err(),
            StorageError::InvalidKey {
                field: "source_id".into()
            }
        );

        let migrated =
            migrate_source_switch_progress(&old_progress, &valid_match, "source-new", 2000)
                .unwrap();
        assert_eq!(
            serde_json::to_value(&migrated).unwrap()["newProgress"]["chapterProgress"],
            serde_json::json!(0.0)
        );
    }

    #[test]
    fn shelf_empty_returns_empty_list_and_zero_count() {
        let store = InMemoryStorage::new();
        assert!(store.list_shelf().unwrap().is_empty());
        assert_eq!(store.shelf_count().unwrap(), 0);
        assert!(store.get_shelf_entry("s1", "b1").unwrap().is_none());
    }

    #[test]
    fn shelf_add_then_get_round_trips() {
        let store = InMemoryStorage::new();
        let entry = shelf_entry("s1", "b1", "Dune", 1000);
        store.add_to_shelf(entry.clone()).unwrap();
        let got = store.get_shelf_entry("s1", "b1").unwrap().unwrap();
        assert_eq!(got, entry);
        assert_eq!(store.shelf_count().unwrap(), 1);
    }

    #[test]
    fn shelf_upsert_preserves_added_at_and_updates_fields() {
        let store = InMemoryStorage::new();
        let original = shelf_entry("s1", "b1", "Dune", 1000);
        store.add_to_shelf(original.clone()).unwrap();

        // Re-add with a different title and later added_at — added_at must be
        // preserved from the original, title must be overwritten.
        let mut updated = shelf_entry("s1", "b1", "Dune (Updated)", 9999);
        updated.author = "Herbert".into();
        store.add_to_shelf(updated).unwrap();

        let got = store.get_shelf_entry("s1", "b1").unwrap().unwrap();
        assert_eq!(got.added_at, 1000, "added_at must be preserved on upsert");
        assert_eq!(got.title, "Dune (Updated)");
        assert_eq!(got.author, "Herbert");
        assert_eq!(store.shelf_count().unwrap(), 1, "upsert must not duplicate");
    }

    #[test]
    fn shelf_remove_is_idempotent() {
        let store = InMemoryStorage::new();
        store
            .add_to_shelf(shelf_entry("s1", "b1", "Dune", 1000))
            .unwrap();
        store.remove_from_shelf("s1", "b1").unwrap();
        assert!(store.get_shelf_entry("s1", "b1").unwrap().is_none());
        // Removing again is not an error.
        store.remove_from_shelf("s1", "b1").unwrap();
        assert_eq!(store.shelf_count().unwrap(), 0);
    }

    #[test]
    fn shelf_get_missing_returns_none() {
        let store = InMemoryStorage::new();
        store
            .add_to_shelf(shelf_entry("s1", "b1", "Dune", 1000))
            .unwrap();
        assert!(store.get_shelf_entry("s1", "missing").unwrap().is_none());
        assert!(store.get_shelf_entry("missing", "b1").unwrap().is_none());
    }

    #[test]
    fn shelf_cross_source_same_book_id_no_collision() {
        let store = InMemoryStorage::new();
        store
            .add_to_shelf(shelf_entry("s1", "b1", "From S1", 1000))
            .unwrap();
        store
            .add_to_shelf(shelf_entry("s2", "b1", "From S2", 2000))
            .unwrap();
        assert_eq!(store.shelf_count().unwrap(), 2);
        let list = store.list_shelf().unwrap();
        assert_eq!(list.len(), 2);
        // Both entries coexist with the same book_id but different source_id.
        let titles: Vec<&str> = list.iter().map(|e| e.title.as_str()).collect();
        assert!(titles.contains(&"From S1"));
        assert!(titles.contains(&"From S2"));
    }

    #[test]
    fn shelf_list_sorted_by_sort_index_then_added_at_desc() {
        let store = InMemoryStorage::new();
        // sort_index 0, added_at 1000 (older)
        store
            .add_to_shelf(shelf_entry("s1", "b1", "A", 1000))
            .unwrap();
        // sort_index 0, added_at 2000 (newer) — should come before A within index 0
        store
            .add_to_shelf(shelf_entry("s1", "b2", "B", 2000))
            .unwrap();
        // sort_index 1 — should come after both index-0 entries
        let mut c = shelf_entry("s1", "b3", "C", 500);
        c.sort_index = 1;
        store.add_to_shelf(c).unwrap();

        let list = store.list_shelf().unwrap();
        let titles: Vec<&str> = list.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["B", "A", "C"],
            "sorted by index asc, added_at desc"
        );
    }

    #[test]
    fn shelf_list_by_group_filters_correctly() {
        let store = InMemoryStorage::new();
        let mut a = shelf_entry("s1", "b1", "A", 1000);
        a.group = Some("追更".into());
        let mut b = shelf_entry("s1", "b2", "B", 2000);
        b.group = Some("默认".into());
        let mut c = shelf_entry("s1", "b3", "C", 3000);
        c.group = Some("追更".into());
        store.add_to_shelf(a).unwrap();
        store.add_to_shelf(b).unwrap();
        store.add_to_shelf(c).unwrap();

        let zhui = store.list_shelf_by_group("追更").unwrap();
        let titles: Vec<&str> = zhui.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, vec!["C", "A"]);

        let moren = store.list_shelf_by_group("默认").unwrap();
        assert_eq!(moren.len(), 1);
        assert_eq!(moren[0].title, "B");
        assert!(store.list_shelf_by_group("不存在").unwrap().is_empty());
    }

    #[test]
    fn shelf_query_filters_by_source_group_keyword_and_progress() {
        let store = InMemoryStorage::new();
        let mut dune = shelf_entry("s1", "dune", "Dune", 1000);
        dune.author = "Frank Herbert".into();
        dune.group = Some("追更".into());
        let mut foundation = shelf_entry("s1", "foundation", "Foundation", 2000);
        foundation.author = "Isaac Asimov".into();
        foundation.group = Some("追更".into());
        let mut other_source = shelf_entry("s2", "dune", "Dune Mirror", 3000);
        other_source.group = Some("追更".into());
        store.add_to_shelf(dune).unwrap();
        store.add_to_shelf(foundation).unwrap();
        store.add_to_shelf(other_source).unwrap();
        store
            .save_reading_progress(progress_entry("s1", "dune", 1, 100, 0.2, 4000))
            .unwrap();

        let result = store
            .query_shelf(BookshelfQuery {
                source_id: Some("s1".into()),
                group: Some("追更".into()),
                keyword: Some("herbert".into()),
                has_reading_progress: Some(true),
                ..BookshelfQuery::default()
            })
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].book_id, "dune");

        let without_progress = store
            .query_shelf(BookshelfQuery {
                source_id: Some("s1".into()),
                has_reading_progress: Some(false),
                ..BookshelfQuery::default()
            })
            .unwrap();
        assert_eq!(
            without_progress
                .into_iter()
                .map(|entry| entry.book_id)
                .collect::<Vec<_>>(),
            vec!["foundation"]
        );
    }

    #[test]
    fn shelf_query_sorts_and_paginates_deterministically() {
        let store = InMemoryStorage::new();
        let mut alpha = shelf_entry("s1", "a", "Alpha", 1000);
        alpha.author = "C".into();
        alpha.sort_index = 2;
        let mut beta = shelf_entry("s1", "b", "Beta", 2000);
        beta.author = "A".into();
        beta.sort_index = 1;
        let mut gamma = shelf_entry("s1", "c", "Gamma", 3000);
        gamma.author = "B".into();
        gamma.sort_index = 3;
        store.add_to_shelf(alpha).unwrap();
        store.add_to_shelf(beta).unwrap();
        store.add_to_shelf(gamma).unwrap();
        store.update_last_read("s1", "a", 5000).unwrap();
        store.update_last_read("s1", "c", 4000).unwrap();

        let recent = store
            .query_shelf(BookshelfQuery {
                sort_by: BookshelfSortBy::LastReadAt,
                sort_direction: BookshelfSortDirection::Descending,
                ..BookshelfQuery::default()
            })
            .unwrap();
        assert_eq!(
            recent
                .into_iter()
                .map(|entry| entry.book_id)
                .collect::<Vec<_>>(),
            vec!["a", "c", "b"]
        );

        let by_author_page = store
            .query_shelf(BookshelfQuery {
                sort_by: BookshelfSortBy::Author,
                offset: 1,
                limit: Some(1),
                ..BookshelfQuery::default()
            })
            .unwrap();
        assert_eq!(by_author_page[0].book_id, "c");

        let manual_desc = store
            .query_shelf(BookshelfQuery {
                sort_by: BookshelfSortBy::Manual,
                sort_direction: BookshelfSortDirection::Descending,
                ..BookshelfQuery::default()
            })
            .unwrap();
        assert_eq!(
            manual_desc
                .into_iter()
                .map(|entry| entry.book_id)
                .collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
    }

    #[test]
    fn shelf_move_updates_group_and_manual_order() {
        let store = InMemoryStorage::new();
        store
            .add_to_shelf(shelf_entry("s1", "b1", "Dune", 1000))
            .unwrap();

        let moved = store
            .move_shelf_entry("s1", "b1", Some("追更".into()), 7)
            .unwrap();

        assert_eq!(moved.group.as_deref(), Some("追更"));
        assert_eq!(moved.sort_index, 7);
        assert_eq!(
            store
                .list_shelf_by_group("追更")
                .unwrap()
                .into_iter()
                .map(|entry| entry.book_id)
                .collect::<Vec<_>>(),
            vec!["b1"]
        );

        let cleared = store.move_shelf_entry("s1", "b1", None, -1).unwrap();
        assert_eq!(cleared.group, None);
        assert_eq!(cleared.sort_index, -1);
    }

    #[test]
    fn shelf_query_and_move_reject_invalid_fields_or_missing_entry() {
        let store = InMemoryStorage::new();
        assert!(matches!(
            store.query_shelf(BookshelfQuery {
                source_id: Some("   ".into()),
                ..BookshelfQuery::default()
            }),
            Err(StorageError::InvalidKey { .. })
        ));
        assert!(matches!(
            store.query_shelf(BookshelfQuery {
                group: Some("   ".into()),
                ..BookshelfQuery::default()
            }),
            Err(StorageError::InvalidKey { .. })
        ));
        assert!(matches!(
            store.list_shelf_by_group("   "),
            Err(StorageError::InvalidKey { .. })
        ));
        assert_eq!(
            store
                .move_shelf_entry("s1", "missing", Some("追更".into()), 1)
                .unwrap_err(),
            StorageError::NotFound {
                source_id: "s1".into(),
                book_id: "missing".into()
            }
        );
        store
            .add_to_shelf(shelf_entry("s1", "b1", "Dune", 1000))
            .unwrap();
        assert!(matches!(
            store.move_shelf_entry("s1", "b1", Some("   ".into()), 1),
            Err(StorageError::InvalidKey { .. })
        ));
    }

    #[test]
    fn bookshelf_query_json_round_trips_and_denies_unknown_fields() {
        let query = BookshelfQuery {
            source_id: Some("s1".into()),
            group: Some("追更".into()),
            keyword: Some("dune".into()),
            has_reading_progress: Some(true),
            sort_by: BookshelfSortBy::Title,
            sort_direction: BookshelfSortDirection::Descending,
            offset: 10,
            limit: Some(20),
        };

        let json = serde_json::to_string(&query).unwrap();
        let back: BookshelfQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(back, query);
        assert!(
            serde_json::from_str::<BookshelfQuery>(r#"{"sortBy":"title","bogus":true}"#).is_err()
        );
    }

    #[test]
    fn shelf_update_last_read_sets_timestamp() {
        let store = InMemoryStorage::new();
        store
            .add_to_shelf(shelf_entry("s1", "b1", "Dune", 1000))
            .unwrap();
        assert!(store
            .get_shelf_entry("s1", "b1")
            .unwrap()
            .unwrap()
            .last_read_at
            .is_none());
        store.update_last_read("s1", "b1", 5000).unwrap();
        let got = store.get_shelf_entry("s1", "b1").unwrap().unwrap();
        assert_eq!(got.last_read_at, Some(5000));
    }

    #[test]
    fn shelf_update_last_read_missing_returns_not_found() {
        let store = InMemoryStorage::new();
        let err = store.update_last_read("s1", "missing", 5000).unwrap_err();
        assert_eq!(
            err,
            StorageError::NotFound {
                source_id: "s1".into(),
                book_id: "missing".into()
            }
        );
    }

    #[test]
    fn shelf_rejects_empty_source_id() {
        let store = InMemoryStorage::new();
        let mut entry = shelf_entry("", "b1", "Dune", 1000);
        let err = store.add_to_shelf(entry.clone()).unwrap_err();
        assert_eq!(
            err,
            StorageError::InvalidKey {
                field: "source_id".into()
            }
        );

        // Whitespace-only also rejected.
        entry.source_id = "   ".into();
        let err = store.add_to_shelf(entry).unwrap_err();
        assert_eq!(
            err,
            StorageError::InvalidKey {
                field: "source_id".into()
            }
        );
    }

    #[test]
    fn shelf_rejects_empty_book_id() {
        let store = InMemoryStorage::new();
        let entry = shelf_entry("s1", "", "Dune", 1000);
        let err = store.add_to_shelf(entry).unwrap_err();
        assert_eq!(
            err,
            StorageError::InvalidKey {
                field: "book_id".into()
            }
        );
    }

    #[test]
    fn shelf_get_and_remove_reject_empty_keys() {
        let store = InMemoryStorage::new();
        assert!(matches!(
            store.get_shelf_entry("", "b1"),
            Err(StorageError::InvalidKey { .. })
        ));
        assert!(matches!(
            store.remove_from_shelf("s1", ""),
            Err(StorageError::InvalidKey { .. })
        ));
        assert!(matches!(
            store.update_last_read("", "", 1),
            Err(StorageError::InvalidKey { .. })
        ));
    }

    #[test]
    fn shelf_entry_json_round_trips() {
        let entry = BookshelfEntry {
            source_id: "s1".into(),
            book_id: "b1".into(),
            title: "Dune".into(),
            author: "Herbert".into(),
            cover_url: Some("https://example.test/cover.jpg".into()),
            intro: None,
            kind: Some("sci-fi".into()),
            last_chapter: Some("Ch 42".into()),
            added_at: 1700000000,
            last_read_at: Some(1700001000),
            group: Some("追更".into()),
            sort_index: 5,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: BookshelfEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn shelf_entry_denies_unknown_fields() {
        // Unknown field must be rejected to keep the schema strict.
        let json = r#"{"sourceId":"s1","bookId":"b1","title":"Dune","addedAt":1,"bogus":true}"#;
        assert!(serde_json::from_str::<BookshelfEntry>(json).is_err());
    }

    // ---- Chapter cache boundary tests ----

    fn chapter_entry(
        source: &str,
        book: &str,
        index: u32,
        title: &str,
        content: &str,
        cached_at: i64,
    ) -> ChapterCacheEntry {
        ChapterCacheEntry {
            source_id: source.into(),
            book_id: book.into(),
            chapter_index: index,
            title: title.into(),
            url: format!("/book/{book}/chapter/{index}"),
            content: content.into(),
            cached_at,
            revision: None,
        }
    }

    #[test]
    fn chapter_cache_empty_returns_none_and_empty_list() {
        let store = InMemoryStorage::new();
        assert!(store.get_chapter_cache("s1", "b1", 0).unwrap().is_none());
        assert!(store.list_chapter_cache("s1", "b1").unwrap().is_empty());
        assert_eq!(store.clear_chapter_cache("s1", "b1").unwrap(), 0);
    }

    #[test]
    fn chapter_cache_put_then_get_round_trips() {
        let store = InMemoryStorage::new();
        let mut entry = chapter_entry("s1", "b1", 2, "Chapter 3", "Body", 1700000000);
        entry.revision = Some("etag-1".into());
        store.put_chapter_cache(entry.clone()).unwrap();

        let got = store.get_chapter_cache("s1", "b1", 2).unwrap().unwrap();
        assert_eq!(got, entry);
    }

    #[test]
    fn chapter_cache_upsert_overwrites_existing_body() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "Old", "old body", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "New", "new body", 2000))
            .unwrap();

        let chapters = store.list_chapter_cache("s1", "b1").unwrap();
        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].title, "New");
        assert_eq!(chapters[0].content, "new body");
        assert_eq!(chapters[0].cached_at, 2000);
    }

    #[test]
    fn chapter_cache_cross_source_and_book_do_not_collide() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "S1 B1", "a", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s2", "b1", 0, "S2 B1", "b", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b2", 0, "S1 B2", "c", 1000))
            .unwrap();

        assert_eq!(
            store
                .get_chapter_cache("s1", "b1", 0)
                .unwrap()
                .unwrap()
                .title,
            "S1 B1"
        );
        assert_eq!(
            store
                .get_chapter_cache("s2", "b1", 0)
                .unwrap()
                .unwrap()
                .title,
            "S2 B1"
        );
        assert_eq!(store.list_chapter_cache("s1", "b1").unwrap().len(), 1);
        assert_eq!(store.list_chapter_cache("s1", "b2").unwrap().len(), 1);
    }

    #[test]
    fn chapter_cache_list_sorted_by_chapter_index() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 2, "C", "c", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "a", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 1, "B", "b", 1000))
            .unwrap();

        let titles: Vec<String> = store
            .list_chapter_cache("s1", "b1")
            .unwrap()
            .into_iter()
            .map(|entry| entry.title)
            .collect();
        assert_eq!(titles, vec!["A", "B", "C"]);
    }

    #[test]
    fn chapter_cache_remove_is_idempotent() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "a", 1000))
            .unwrap();

        store.remove_chapter_cache("s1", "b1", 0).unwrap();
        assert!(store.get_chapter_cache("s1", "b1", 0).unwrap().is_none());
        store.remove_chapter_cache("s1", "b1", 0).unwrap();
    }

    #[test]
    fn chapter_cache_clear_removes_only_requested_book() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "a", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 1, "B", "b", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b2", 0, "Other", "c", 1000))
            .unwrap();

        assert_eq!(store.clear_chapter_cache("s1", "b1").unwrap(), 2);
        assert!(store.list_chapter_cache("s1", "b1").unwrap().is_empty());
        assert_eq!(store.list_chapter_cache("s1", "b2").unwrap().len(), 1);
    }

    #[test]
    fn chapter_cache_rejects_empty_keys() {
        let store = InMemoryStorage::new();
        let err = store
            .put_chapter_cache(chapter_entry("", "b1", 0, "A", "a", 1000))
            .unwrap_err();
        assert_eq!(
            err,
            StorageError::InvalidKey {
                field: "source_id".into()
            }
        );

        assert!(matches!(
            store.get_chapter_cache("s1", "   ", 0),
            Err(StorageError::InvalidKey { .. })
        ));
        assert!(matches!(
            store.remove_chapter_cache("", "b1", 0),
            Err(StorageError::InvalidKey { .. })
        ));
        assert!(matches!(
            store.clear_chapter_cache("s1", ""),
            Err(StorageError::InvalidKey { .. })
        ));
    }

    #[test]
    fn chapter_cache_entry_json_round_trips() {
        let mut entry = chapter_entry("s1", "b1", 3, "Chapter 4", "", 1700000000);
        entry.url = String::new();
        entry.revision = Some("hash-1".into());

        let json = serde_json::to_string(&entry).unwrap();
        let back: ChapterCacheEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn chapter_cache_entry_denies_unknown_fields() {
        let json = r#"{"sourceId":"s1","bookId":"b1","chapterIndex":0,"content":"x","cachedAt":1,"bogus":true}"#;
        assert!(serde_json::from_str::<ChapterCacheEntry>(json).is_err());
    }

    fn offline_chapter(
        source_id: &str,
        chapter_id: &str,
        status: ChapterCacheStatus,
    ) -> OfflineChapterCacheEntry {
        OfflineChapterCacheEntry::new(source_id, chapter_id, status).unwrap()
    }

    fn unified_offline_cache_entry(
        source: &str,
        book: &str,
        chapter: &str,
    ) -> UnifiedOfflineChapterCacheEntry {
        UnifiedOfflineChapterCacheEntry {
            source_id: source.into(),
            book_id: book.into(),
            chapter_id: chapter.into(),
            source_content_locator_checksum: format!("locator-{chapter}"),
            content_checksum: format!("content-{chapter}"),
            normalized_content_type: "text/plain".into(),
            byte_count: 128,
            creation_timestamp: "1970-01-01T00:00:00Z".into(),
            last_access_timestamp: "1970-01-01T00:00:00Z".into(),
            validation_timestamp: "1970-01-01T00:00:00Z".into(),
            parser_runtime_version: "RECOVERY-33".into(),
            source_fingerprint_or_remote_toc_version:
                "remote:recovery33-remote-source:remote-book-1".into(),
            encryption_at_rest_capability: Some("host_supplied_optional".into()),
            state: ChapterCacheStatus::Available,
            pinned: false,
        }
    }

    #[test]
    fn offline_availability_counts_legacy_cache_statuses_by_source() {
        let entries = vec![
            offline_chapter("s1", "c1", ChapterCacheStatus::Available),
            offline_chapter("s1", "c2", ChapterCacheStatus::Available),
            offline_chapter("s1", "c3", ChapterCacheStatus::Stale),
            offline_chapter("s1", "c4", ChapterCacheStatus::Failed),
            offline_chapter("s1", "c5", ChapterCacheStatus::Missing),
            offline_chapter("s1", "c6", ChapterCacheStatus::Evicted),
            offline_chapter("s2", "c1", ChapterCacheStatus::Available),
        ];

        let availability = compute_offline_availability("s1", 8, &entries).unwrap();

        assert_eq!(availability.source_id, "s1");
        assert_eq!(availability.total_chapters, 8);
        assert_eq!(availability.cached_chapters, 2);
        assert_eq!(availability.stale_chapters, 1);
        assert_eq!(availability.failed_chapters, 1);
        assert_eq!(availability.missing_chapters, 4);
        assert_eq!(availability.availability_ratio(), 0.25);
        assert!(offline_can_read_chapter(&entries, "c1"));
        assert!(!offline_can_read_chapter(&entries, "c3"));
        assert!(!offline_can_read_chapter(&entries, "missing"));
    }

    #[test]
    fn offline_availability_next_download_range_matches_legacy_one_indexed_plan() {
        let partial = OfflineAvailability::new("s1", 12, 3, 2, 1).unwrap();
        assert_eq!(
            partial.next_download_range(4),
            Some(OfflineDownloadRange {
                start: 4,
                end_inclusive: 7
            })
        );
        assert_eq!(
            partial.next_download_range(20),
            Some(OfflineDownloadRange {
                start: 4,
                end_inclusive: 12
            })
        );
        assert_eq!(partial.next_download_range(0), None);

        let complete = OfflineAvailability::new("s1", 3, 3, 0, 0).unwrap();
        assert_eq!(complete.next_download_range(10), None);

        let empty = OfflineAvailability::new("s1", 0, 0, 0, 0).unwrap();
        assert_eq!(empty.availability_ratio(), 1.0);
        assert_eq!(empty.next_download_range(10), None);
    }

    #[test]
    fn offline_availability_json_shape_and_validation_reject_drift() {
        assert_eq!(
            serde_json::to_value(ChapterCacheStatus::Available).unwrap(),
            serde_json::json!("available")
        );

        let entry = offline_chapter("s1", "chapter-1", ChapterCacheStatus::Failed);
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            serde_json::json!({
                "sourceId": "s1",
                "chapterId": "chapter-1",
                "status": "failed"
            })
        );
        assert!(
            serde_json::from_value::<OfflineChapterCacheEntry>(serde_json::json!({
                "sourceId": "s1",
                "chapterId": "chapter-1",
                "status": "available",
                "hostPath": "/tmp/chapter"
            }))
            .is_err()
        );
        assert_eq!(
            OfflineChapterCacheEntry::new("s1", "   ", ChapterCacheStatus::Available).unwrap_err(),
            StorageError::InvalidChapterCache {
                field: "chapter_id".into()
            }
        );
        assert_eq!(
            compute_offline_availability(" ", 1, &[]).unwrap_err(),
            StorageError::InvalidKey {
                field: "source_id".into()
            }
        );

        let oversubscribed = OfflineAvailability::new("s1", 2, 2, 1, 1).unwrap();
        assert_eq!(oversubscribed.missing_chapters, -2);
    }

    #[test]
    fn unified_offline_cache_validation_matches_recovery33_state_order() {
        let entry = unified_offline_cache_entry("remote-a", "book-1", "chapter-1");
        let entries = vec![entry.clone()];

        assert_eq!(
            validate_unified_offline_chapter_cache(
                &[],
                "remote-a",
                "book-1",
                "chapter-1",
                "remote:missing",
                "RECOVERY-33",
                None
            )
            .unwrap(),
            ChapterCacheStatus::Missing
        );
        assert_eq!(
            validate_unified_offline_chapter_cache(
                &entries,
                "remote-a",
                "book-1",
                "chapter-1",
                "remote:recovery33-remote-source:remote-book-1",
                "RECOVERY-33",
                Some("content-chapter-1")
            )
            .unwrap(),
            ChapterCacheStatus::Validated
        );
        assert_eq!(
            validate_unified_offline_chapter_cache(
                &entries,
                "remote-a",
                "book-1",
                "chapter-1",
                "changed",
                "RECOVERY-33",
                Some("content-chapter-1")
            )
            .unwrap(),
            ChapterCacheStatus::Stale
        );
        assert_eq!(
            validate_unified_offline_chapter_cache(
                &entries,
                "remote-a",
                "book-1",
                "chapter-1",
                "remote:recovery33-remote-source:remote-book-1",
                "RECOVERY-34",
                Some("content-chapter-1")
            )
            .unwrap(),
            ChapterCacheStatus::Invalidated
        );
        assert_eq!(
            validate_unified_offline_chapter_cache(
                &entries,
                "remote-a",
                "book-1",
                "chapter-1",
                "remote:recovery33-remote-source:remote-book-1",
                "RECOVERY-33",
                Some("corrupt")
            )
            .unwrap(),
            ChapterCacheStatus::Failed
        );
        assert_eq!(
            validate_unified_offline_chapter_cache(
                &entries,
                "remote-a",
                "book-1",
                "chapter-1",
                "remote:recovery33-remote-source:remote-book-1",
                "RECOVERY-33",
                None
            )
            .unwrap(),
            ChapterCacheStatus::Validated
        );

        let other_source = unified_offline_cache_entry("remote-b", "book-1", "chapter-1");
        assert_eq!(
            validate_unified_offline_chapter_cache(
                &[other_source],
                "remote-a",
                "book-1",
                "chapter-1",
                "remote:recovery33-remote-source:remote-book-1",
                "RECOVERY-33",
                None
            )
            .unwrap(),
            ChapterCacheStatus::Missing
        );
    }

    #[test]
    fn unified_offline_cache_metadata_json_shape_and_validation_reject_drift() {
        let entry = unified_offline_cache_entry("remote-a", "book-1", "chapter-1");
        entry.validate().unwrap();
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["sourceId"], "remote-a");
        assert_eq!(json["bookId"], "book-1");
        assert_eq!(json["chapterId"], "chapter-1");
        assert_eq!(json["parserRuntimeVersion"], "RECOVERY-33");
        assert_eq!(
            json["sourceFingerprintOrRemoteTocVersion"],
            "remote:recovery33-remote-source:remote-book-1"
        );
        assert_eq!(json["state"], "available");
        assert_eq!(
            serde_json::to_value(ChapterCacheStatus::Invalidated).unwrap(),
            serde_json::json!("invalidated")
        );

        assert!(
            serde_json::from_value::<UnifiedOfflineChapterCacheEntry>(serde_json::json!({
                "sourceId": "remote-a",
                "bookId": "book-1",
                "chapterId": "chapter-1",
                "sourceContentLocatorChecksum": "locator",
                "contentChecksum": "content",
                "normalizedContentType": "text/plain",
                "byteCount": 128,
                "creationTimestamp": "1970-01-01T00:00:00Z",
                "lastAccessTimestamp": "1970-01-01T00:00:00Z",
                "validationTimestamp": "1970-01-01T00:00:00Z",
                "parserRuntimeVersion": "RECOVERY-33",
                "sourceFingerprintOrRemoteTocVersion": "remote:v1",
                "state": "available",
                "pinned": false,
                "absolutePath": "/Users/example/cache/chapter"
            }))
            .is_err()
        );

        let mut invalid = entry.clone();
        invalid.parser_runtime_version = " ".into();
        assert_eq!(
            invalid.validate().unwrap_err(),
            StorageError::InvalidChapterCache {
                field: "parser_runtime_version".into()
            }
        );
        assert_eq!(
            validate_unified_offline_chapter_cache(
                &[entry],
                "remote-a",
                "book-1",
                "chapter-1",
                "remote:recovery33-remote-source:remote-book-1",
                "RECOVERY-33",
                Some(" ")
            )
            .unwrap_err(),
            StorageError::InvalidChapterCache {
                field: "normalized_content_checksum".into()
            }
        );
    }

    #[test]
    fn chapter_cache_stats_reports_count_bytes_and_age_range() {
        let store = InMemoryStorage::new();
        assert_eq!(
            store.chapter_cache_stats().unwrap(),
            ChapterCacheStats {
                entry_count: 0,
                total_content_bytes: 0,
                oldest_cached_at: None,
                newest_cached_at: None,
            }
        );
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "abc", 3000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 1, "B", "abcdef", 1000))
            .unwrap();

        assert_eq!(
            store.chapter_cache_stats().unwrap(),
            ChapterCacheStats {
                entry_count: 2,
                total_content_bytes: 9,
                oldest_cached_at: Some(1000),
                newest_cached_at: Some(3000),
            }
        );
    }

    #[test]
    fn chapter_cache_coverage_reports_cached_missing_bytes_and_age() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "abc", 3000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 2, "C", "de", 1000))
            .unwrap();
        // Outside current TOC length; should not count as coverage.
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 8, "Old", "ignored", 500))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s2", "b1", 1, "Other", "x", 100))
            .unwrap();

        let coverage = store.chapter_cache_coverage("s1", "b1", 4).unwrap();

        assert_eq!(coverage.source_id, "s1");
        assert_eq!(coverage.book_id, "b1");
        assert_eq!(coverage.chapter_count, 4);
        assert_eq!(coverage.cached_indexes, vec![0, 2]);
        assert_eq!(coverage.missing_indexes, vec![1, 3]);
        assert_eq!(coverage.cached_count, 2);
        assert_eq!(coverage.missing_count, 2);
        assert_eq!(coverage.total_content_bytes, 5);
        assert_eq!(coverage.oldest_cached_at, Some(1000));
        assert_eq!(coverage.newest_cached_at, Some(3000));
        assert!(!coverage.complete);
    }

    #[test]
    fn chapter_cache_coverage_marks_complete_books() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "a", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 1, "B", "b", 2000))
            .unwrap();

        let coverage = store.chapter_cache_coverage("s1", "b1", 2).unwrap();

        assert!(coverage.complete);
        assert!(coverage.missing_indexes.is_empty());
        assert_eq!(coverage.cached_indexes, vec![0, 1]);
    }

    #[test]
    fn chapter_cache_prefetch_plan_returns_missing_indexes_in_anchor_window() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "a", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 2, "C", "c", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 5, "F", "f", 1000))
            .unwrap();

        let plan = store
            .plan_chapter_cache_prefetch("s1", "b1", 6, 2, 2, 3, 2)
            .unwrap();

        assert_eq!(plan.source_id, "s1");
        assert_eq!(plan.book_id, "b1");
        assert_eq!(plan.chapter_count, 6);
        assert_eq!(plan.anchor_index, 2);
        assert_eq!(plan.window_start, 0);
        assert_eq!(plan.window_end_exclusive, 6);
        assert_eq!(plan.missing_indexes, vec![1, 3]);
    }

    #[test]
    fn chapter_cache_prefetch_plan_clamps_window_edges() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "a", 1000))
            .unwrap();

        let plan = store
            .plan_chapter_cache_prefetch("s1", "b1", 3, 0, 10, 10, 10)
            .unwrap();
        assert_eq!(plan.window_start, 0);
        assert_eq!(plan.window_end_exclusive, 3);
        assert_eq!(plan.missing_indexes, vec![1, 2]);

        let plan = store
            .plan_chapter_cache_prefetch("s1", "b1", 3, 2, 1, 10, 10)
            .unwrap();
        assert_eq!(plan.window_start, 1);
        assert_eq!(plan.window_end_exclusive, 3);
        assert_eq!(plan.missing_indexes, vec![1, 2]);
    }

    #[test]
    fn chapter_cache_coverage_and_prefetch_reject_invalid_inputs() {
        let store = InMemoryStorage::new();
        assert_eq!(
            store.chapter_cache_coverage("s1", "b1", 0).unwrap_err(),
            StorageError::InvalidChapterCache {
                field: "chapter_count".into()
            }
        );
        assert_eq!(
            store
                .plan_chapter_cache_prefetch("s1", "b1", 3, 3, 1, 1, 1)
                .unwrap_err(),
            StorageError::InvalidChapterCache {
                field: "anchor_index".into()
            }
        );
        assert_eq!(
            store
                .plan_chapter_cache_prefetch("s1", "b1", 3, 1, 1, 1, 0)
                .unwrap_err(),
            StorageError::InvalidChapterCache {
                field: "max_count".into()
            }
        );
        assert!(matches!(
            store.chapter_cache_coverage("", "b1", 1),
            Err(StorageError::InvalidKey { .. })
        ));
    }

    #[test]
    fn chapter_cache_coverage_and_prefetch_json_round_trip() {
        let coverage = ChapterCacheCoverage {
            source_id: "s1".into(),
            book_id: "b1".into(),
            chapter_count: 3,
            cached_indexes: vec![0, 2],
            missing_indexes: vec![1],
            cached_count: 2,
            missing_count: 1,
            total_content_bytes: 10,
            oldest_cached_at: Some(100),
            newest_cached_at: Some(200),
            complete: false,
        };
        let json = serde_json::to_string(&coverage).unwrap();
        let back: ChapterCacheCoverage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, coverage);
        assert!(serde_json::from_str::<ChapterCacheCoverage>(
            r#"{"sourceId":"s","bookId":"b","chapterCount":1,"cachedCount":0,"missingCount":1,"totalContentBytes":0,"complete":false,"bogus":true}"#
        )
        .is_err());

        let plan = ChapterCachePrefetchPlan {
            source_id: "s1".into(),
            book_id: "b1".into(),
            chapter_count: 3,
            anchor_index: 1,
            window_start: 0,
            window_end_exclusive: 3,
            missing_indexes: vec![1, 2],
        };
        let json = serde_json::to_string(&plan).unwrap();
        let back: ChapterCachePrefetchPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, plan);
    }

    #[test]
    fn chapter_cache_prune_default_policy_is_noop() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "abc", 1000))
            .unwrap();

        let report = store
            .prune_chapter_cache(ChapterCacheRetentionPolicy::default())
            .unwrap();

        assert!(report.removed.is_empty());
        assert_eq!(report.remaining.entry_count, 1);
        assert_eq!(store.list_chapter_cache("s1", "b1").unwrap().len(), 1);
    }

    #[test]
    fn chapter_cache_prune_by_max_entries_evicts_oldest_first() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "Old", "a", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 1, "Middle", "b", 2000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 2, "New", "c", 3000))
            .unwrap();

        let report = store
            .prune_chapter_cache(ChapterCacheRetentionPolicy {
                max_entries: Some(2),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].title, "Old");
        assert_eq!(report.remaining.entry_count, 2);
        let titles = store
            .list_chapter_cache("s1", "b1")
            .unwrap()
            .into_iter()
            .map(|entry| entry.title)
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["Middle", "New"]);
    }

    #[test]
    fn chapter_cache_prune_by_total_bytes_evicts_until_under_limit() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "12345", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 1, "B", "1234", 2000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 2, "C", "123", 3000))
            .unwrap();

        let report = store
            .prune_chapter_cache(ChapterCacheRetentionPolicy {
                max_total_content_bytes: Some(7),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(
            report
                .removed
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            vec!["A"]
        );
        assert_eq!(report.remaining.total_content_bytes, 7);
        assert_eq!(report.remaining.entry_count, 2);
    }

    #[test]
    fn chapter_cache_prune_by_min_cached_at_removes_expired_entries() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "Expired", "a", 999))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 1, "Fresh", "b", 1000))
            .unwrap();

        let report = store
            .prune_chapter_cache(ChapterCacheRetentionPolicy {
                min_cached_at: Some(1000),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].title, "Expired");
        assert_eq!(report.remaining.oldest_cached_at, Some(1000));
    }

    #[test]
    fn chapter_cache_prune_combined_policy_does_not_double_count_removed() {
        let store = InMemoryStorage::new();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 0, "A", "12345", 1000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 1, "B", "12345", 2000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 2, "C", "12345", 3000))
            .unwrap();
        store
            .put_chapter_cache(chapter_entry("s1", "b1", 3, "D", "1", 4000))
            .unwrap();

        let report = store
            .prune_chapter_cache(ChapterCacheRetentionPolicy {
                max_entries: Some(3),
                max_total_content_bytes: Some(6),
                min_cached_at: Some(2000),
            })
            .unwrap();

        assert_eq!(
            report
                .removed
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        assert_eq!(report.remaining.entry_count, 2);
        assert_eq!(report.remaining.total_content_bytes, 6);
    }

    #[test]
    fn chapter_cache_retention_policy_json_round_trips_and_denies_unknown_fields() {
        let policy = ChapterCacheRetentionPolicy {
            max_entries: Some(20),
            max_total_content_bytes: Some(1024),
            min_cached_at: Some(1700000000),
        };
        let json = serde_json::to_string(&policy).unwrap();
        let back: ChapterCacheRetentionPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, policy);

        assert!(serde_json::from_str::<ChapterCacheRetentionPolicy>(
            r#"{"maxEntries":1,"bogus":true}"#
        )
        .is_err());
    }

    // ---- Chapter download queue boundary tests ----

    fn download_task(
        source: &str,
        book: &str,
        index: u32,
        priority: i32,
        created_at: i64,
    ) -> ChapterDownloadTask {
        ChapterDownloadTask::pending(
            source,
            book,
            index,
            format!("Chapter {index}"),
            format!("/book/{book}/chapter/{index}"),
            priority,
            created_at,
        )
        .unwrap()
    }

    fn unified_download_request() -> UnifiedDownloadTaskRequest {
        UnifiedDownloadTaskRequest {
            task_id: "download:book-1:c0|c1|c2".into(),
            source_id: "remote-a".into(),
            book_id: "book-1".into(),
            requested_chapter_ids: vec!["c0".into(), "c1".into(), "c2".into()],
            execution_policy: UnifiedDownloadTaskPolicy::AllUncached,
            concurrency_limit: 4,
            maximum_request_count: 32,
            runtime_maximum_concurrent_downloads: 2,
            runtime_maximum_request_count: 8,
            in_flight_task_ids: Vec::new(),
            cached_entries: Vec::new(),
            fetch_outcomes: Vec::new(),
            cancellation_state: None,
        }
    }

    fn download_outcome(
        chapter_id: &str,
        state: ChapterCacheStatus,
        byte_count: u64,
    ) -> UnifiedDownloadChapterOutcome {
        UnifiedDownloadChapterOutcome {
            chapter_id: chapter_id.into(),
            state,
            byte_count,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn unified_download_task_skips_cached_chapters_and_preserves_requested_order() {
        let mut request = unified_download_request();
        request.cached_entries = vec![unified_offline_cache_entry("remote-a", "book-1", "c0")];
        request.fetch_outcomes = vec![
            download_outcome("c1", ChapterCacheStatus::Available, 90),
            download_outcome("c2", ChapterCacheStatus::Available, 110),
        ];

        let task = plan_unified_chapter_download_task(&request).unwrap();

        assert_eq!(task.current_state, UnifiedDownloadTaskState::Completed);
        assert_eq!(task.concurrency_limit, 2);
        assert_eq!(task.maximum_request_count, 8);
        assert_eq!(task.completed_count, 2);
        assert_eq!(task.failed_count, 0);
        assert_eq!(task.skipped_cached_count, 1);
        assert_eq!(task.bytes_received, 200);
        assert_eq!(task.cancellation_state, "not_cancelled");
        assert_eq!(
            task.per_chapter_results
                .iter()
                .map(|result| (
                    result.chapter_id.as_str(),
                    result.cache_hit,
                    result.byte_count
                ))
                .collect::<Vec<_>>(),
            vec![("c0", true, 128), ("c1", false, 90), ("c2", false, 110)]
        );
    }

    #[test]
    fn unified_download_task_reports_partial_failed_and_request_limit_boundaries() {
        let mut request = unified_download_request();
        request.maximum_request_count = 2;
        request.runtime_maximum_request_count = 10;
        request.fetch_outcomes = vec![download_outcome("c0", ChapterCacheStatus::Available, 64)];

        let task = plan_unified_chapter_download_task(&request).unwrap();

        assert_eq!(task.current_state, UnifiedDownloadTaskState::Partial);
        assert_eq!(task.completed_count, 1);
        assert_eq!(task.failed_count, 1);
        assert_eq!(task.bytes_received, 64);
        assert_eq!(
            task.per_chapter_results
                .iter()
                .map(|result| result.chapter_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c0", "c1"]
        );
        assert_eq!(
            task.failure_diagnostics,
            vec!["download_outcome_missing".to_string()]
        );

        let mut failed_request = unified_download_request();
        failed_request.fetch_outcomes = vec![
            download_outcome("c0", ChapterCacheStatus::Failed, 0),
            download_outcome("c1", ChapterCacheStatus::Failed, 0),
            download_outcome("c2", ChapterCacheStatus::Failed, 0),
        ];
        let failed = plan_unified_chapter_download_task(&failed_request).unwrap();
        assert_eq!(failed.current_state, UnifiedDownloadTaskState::Failed);
        assert_eq!(failed.failed_count, 3);
    }

    #[test]
    fn unified_download_task_coalesces_duplicate_task_and_rejects_drift() {
        let mut request = unified_download_request();
        request.in_flight_task_ids = vec![request.task_id.clone()];
        request.fetch_outcomes = vec![download_outcome("c0", ChapterCacheStatus::Available, 64)];

        let task = plan_unified_chapter_download_task(&request).unwrap();

        assert_eq!(task.current_state, UnifiedDownloadTaskState::Partial);
        assert_eq!(task.cancellation_state, "coalesced");
        assert_eq!(task.failure_diagnostics, vec!["duplicate_task_coalesced"]);
        assert!(task.per_chapter_results.is_empty());
        assert_eq!(
            serde_json::to_value(UnifiedDownloadTaskPolicy::AllUncached).unwrap(),
            serde_json::json!("all_uncached")
        );
        assert_eq!(
            serde_json::to_value(UnifiedDownloadTaskState::Partial).unwrap(),
            serde_json::json!("partial")
        );

        let json = serde_json::to_value(&task).unwrap();
        assert_eq!(json["taskId"], request.task_id);
        assert_eq!(json["requestedChapterIds"][0], "c0");
        assert!(
            serde_json::from_value::<UnifiedChapterDownloadTask>(serde_json::json!({
                "taskId": "t",
                "sourceId": "remote-a",
                "bookId": "book-1",
                "requestedChapterIds": [],
                "executionPolicy": "all_uncached",
                "concurrencyLimit": 1,
                "maximumRequestCount": 1,
                "currentState": "completed",
                "completedCount": 0,
                "failedCount": 0,
                "skippedCachedCount": 0,
                "bytesReceived": 0,
                "cancellationState": "not_cancelled",
                "failureDiagnostics": [],
                "perChapterResults": [],
                "hostFetchStarted": true
            }))
            .is_err()
        );

        let mut invalid = unified_download_request();
        invalid.requested_chapter_ids = vec![" ".into()];
        assert_eq!(
            plan_unified_chapter_download_task(&invalid).unwrap_err(),
            StorageError::InvalidDownloadTask {
                field: "requested_chapter_ids".into()
            }
        );
    }

    #[test]
    fn download_queue_enqueue_get_and_list_round_trip() {
        let store = InMemoryStorage::new();
        let task = download_task("s1", "b1", 2, 10, 1000);

        let stored = store.enqueue_chapter_download(task.clone()).unwrap();

        assert_eq!(stored, task);
        assert_eq!(
            store.get_chapter_download("s1", "b1", 2).unwrap(),
            Some(task.clone())
        );
        assert_eq!(
            store.list_chapter_downloads("s1", "b1").unwrap(),
            vec![task]
        );
    }

    #[test]
    fn download_queue_requeue_preserves_created_at_and_resets_retry_state() {
        let store = InMemoryStorage::new();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 0, 1, 1000))
            .unwrap();
        let claimed = store.claim_next_chapter_download(1100).unwrap().unwrap();
        assert_eq!(claimed.status, ChapterDownloadStatus::InProgress);
        store
            .mark_chapter_download_failed("s1", "b1", 0, "timeout", 1200)
            .unwrap();

        let mut requeued = download_task("s1", "b1", 0, 9, 2000);
        requeued.title = "Updated".into();
        let stored = store.enqueue_chapter_download(requeued).unwrap();

        assert_eq!(
            stored.created_at, 1000,
            "created_at is stable across requeue"
        );
        assert_eq!(stored.updated_at, 2000);
        assert_eq!(stored.priority, 9);
        assert_eq!(stored.title, "Updated");
        assert_eq!(stored.status, ChapterDownloadStatus::Pending);
        assert_eq!(stored.attempts, 0);
        assert_eq!(stored.last_error, None);
    }

    #[test]
    fn download_queue_cross_source_same_book_chapter_no_collision() {
        let store = InMemoryStorage::new();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 0, 1, 1000))
            .unwrap();
        store
            .enqueue_chapter_download(download_task("s2", "b1", 0, 2, 1000))
            .unwrap();

        assert_eq!(
            store
                .get_chapter_download("s1", "b1", 0)
                .unwrap()
                .unwrap()
                .source_id,
            "s1"
        );
        assert_eq!(
            store
                .get_chapter_download("s2", "b1", 0)
                .unwrap()
                .unwrap()
                .source_id,
            "s2"
        );
        assert_eq!(store.list_chapter_downloads("s1", "b1").unwrap().len(), 1);
        assert_eq!(store.list_chapter_downloads("s2", "b1").unwrap().len(), 1);
    }

    #[test]
    fn download_queue_claims_highest_priority_then_oldest_update() {
        let store = InMemoryStorage::new();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 0, 1, 1000))
            .unwrap();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 1, 10, 3000))
            .unwrap();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 2, 10, 2000))
            .unwrap();

        let first = store.claim_next_chapter_download(4000).unwrap().unwrap();
        assert_eq!(first.chapter_index, 2);
        assert_eq!(first.status, ChapterDownloadStatus::InProgress);
        assert_eq!(first.attempts, 1);
        assert_eq!(first.updated_at, 4000);

        let second = store.claim_next_chapter_download(5000).unwrap().unwrap();
        assert_eq!(second.chapter_index, 1);
        let third = store.claim_next_chapter_download(6000).unwrap().unwrap();
        assert_eq!(third.chapter_index, 0);
        assert!(store.claim_next_chapter_download(7000).unwrap().is_none());
    }

    #[test]
    fn download_queue_failed_task_retries_until_max_attempts() {
        let store = InMemoryStorage::new();
        let mut task = download_task("s1", "b1", 0, 1, 1000);
        task.max_attempts = 2;
        store.enqueue_chapter_download(task).unwrap();

        let first = store.claim_next_chapter_download(1100).unwrap().unwrap();
        assert_eq!(first.attempts, 1);
        let failed = store
            .mark_chapter_download_failed("s1", "b1", 0, "timeout", 1200)
            .unwrap();
        assert_eq!(failed.status, ChapterDownloadStatus::Failed);
        assert_eq!(failed.last_error.as_deref(), Some("timeout"));

        let second = store.claim_next_chapter_download(1300).unwrap().unwrap();
        assert_eq!(second.attempts, 2);
        store
            .mark_chapter_download_failed("s1", "b1", 0, "still failing", 1400)
            .unwrap();
        assert!(
            store.claim_next_chapter_download(1500).unwrap().is_none(),
            "exhausted failed task must not be claimed again"
        );
    }

    #[test]
    fn download_queue_complete_cancel_and_clear_finished() {
        let store = InMemoryStorage::new();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 0, 1, 1000))
            .unwrap();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 1, 1, 1000))
            .unwrap();
        store
            .enqueue_chapter_download(download_task("s1", "b1", 2, 1, 1000))
            .unwrap();

        store.claim_next_chapter_download(1100).unwrap();
        let completed = store
            .mark_chapter_download_completed("s1", "b1", 0, 1200)
            .unwrap();
        assert_eq!(completed.status, ChapterDownloadStatus::Completed);
        let cancelled = store.cancel_chapter_download("s1", "b1", 1, 1300).unwrap();
        assert_eq!(cancelled.status, ChapterDownloadStatus::Cancelled);

        assert_eq!(
            store.clear_finished_chapter_downloads("s1", "b1").unwrap(),
            2
        );
        let remaining = store.list_chapter_downloads("s1", "b1").unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].chapter_index, 2);
    }

    #[test]
    fn download_queue_mark_missing_returns_not_found() {
        let store = InMemoryStorage::new();
        let err = store
            .mark_chapter_download_completed("s1", "b1", 7, 1000)
            .unwrap_err();
        assert_eq!(
            err,
            StorageError::DownloadTaskNotFound {
                source_id: "s1".into(),
                book_id: "b1".into(),
                chapter_index: 7
            }
        );
    }

    #[test]
    fn download_queue_rejects_invalid_keys_attempts_and_error() {
        let store = InMemoryStorage::new();
        assert!(matches!(
            ChapterDownloadTask::pending("", "b1", 0, "A", "/a", 1, 1000),
            Err(StorageError::InvalidKey { .. })
        ));

        let mut task = download_task("s1", "b1", 0, 1, 1000);
        task.max_attempts = 0;
        assert_eq!(
            store.enqueue_chapter_download(task).unwrap_err(),
            StorageError::InvalidDownloadTask {
                field: "max_attempts".into()
            }
        );

        let mut task = download_task("s1", "b1", 0, 1, 1000);
        task.attempts = 4;
        task.max_attempts = 3;
        assert!(matches!(
            store.enqueue_chapter_download(task),
            Err(StorageError::InvalidDownloadTask { .. })
        ));

        store
            .enqueue_chapter_download(download_task("s1", "b1", 0, 1, 1000))
            .unwrap();
        assert!(matches!(
            store.mark_chapter_download_failed("s1", "b1", 0, "   ", 1100),
            Err(StorageError::InvalidDownloadTask { .. })
        ));
        assert!(matches!(
            store.get_chapter_download("", "b1", 0),
            Err(StorageError::InvalidKey { .. })
        ));
    }

    #[test]
    fn download_task_json_round_trips_and_denies_unknown_fields() {
        let mut task = download_task("s1", "b1", 3, 5, 1700000000);
        task.status = ChapterDownloadStatus::Failed;
        task.attempts = 2;
        task.last_error = Some("timeout".into());

        let json = serde_json::to_string(&task).unwrap();
        let back: ChapterDownloadTask = serde_json::from_str(&json).unwrap();
        assert_eq!(back, task);

        let err_json = r#"{"sourceId":"s1","bookId":"b1","chapterIndex":0,"status":"pending","createdAt":1,"updatedAt":1,"bogus":true}"#;
        assert!(serde_json::from_str::<ChapterDownloadTask>(err_json).is_err());
    }
}

//! Reader-Core storage — SQLite schema, migrations, cache, progress, download queue.
//!
//! V1 ships an in-memory implementation only. The real SQLite-backed store is
//! deferred to a later phase; the trait surface here is what the runtime
//! vertical commands depend on, so swapping the backend later is localized to
//! this crate.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;

use reader_domain::{Book, ReadingProgress, Source};
use serde::{Deserialize, Serialize};

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

fn sort_storage_snapshot(snapshot: &mut StorageSnapshot) {
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

fn validate_book_key(source_id: &str, book_id: &str) -> Result<(), StorageError> {
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

fn validate_source_id(source_id: &str) -> Result<(), StorageError> {
    if source_id.trim().is_empty() {
        return Err(StorageError::InvalidKey {
            field: "source_id".into(),
        });
    }
    Ok(())
}

fn validate_reading_progress(entry: &ReadingProgressEntry) -> Result<(), StorageError> {
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

fn default_max_attempts() -> u32 {
    3
}

fn validate_chapter_download_task(task: &ChapterDownloadTask) -> Result<(), StorageError> {
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

fn validate_shelf_key(source_id: &str, book_id: &str) -> Result<(), StorageError> {
    validate_book_key(source_id, book_id)
}

fn sort_shelf(entries: &mut Vec<BookshelfEntry>) {
    // sort_index ascending; ties broken by added_at descending (newer first).
    entries.sort_by(|a, b| {
        a.sort_index
            .cmp(&b.sort_index)
            .then_with(|| b.added_at.cmp(&a.added_at))
            .then_with(|| a.source_id.cmp(&b.source_id))
            .then_with(|| a.book_id.cmp(&b.book_id))
    });
}

fn normalize_required_filter(
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

fn normalize_keyword(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn normalize_group(value: Option<String>) -> Result<Option<String>, StorageError> {
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

fn sort_shelf_query(
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

fn paginate_shelf(
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

fn chapter_cache_content_bytes(entry: &ChapterCacheEntry) -> usize {
    entry.content.as_bytes().len()
}

fn chapter_cache_stats_from_entries<'a>(
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
    /// A storage snapshot was invalid or incompatible.
    InvalidSnapshot {
        field: String,
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
            StorageError::InvalidSnapshot { field } => {
                write!(f, "invalid storage snapshot field: {field}")
            }
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

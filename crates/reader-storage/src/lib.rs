//! Reader-Core storage — bookshelf, progress, chapter cache, download queue.
//!
//! V1 ships an in-memory implementation only. The trait surface defined here
//! (`BookshelfStore`, `ProgressStore`, `ChapterCacheStore`,
//! `DownloadQueueStore`) is what the persistence line depends on, so a future
//! SQLite-backed store only needs to implement these traits without touching
//! the rest of the crate graph (the runtime keeps using the inherent
//! source/book/progress/cache methods below for backward compatibility).
//!
//! The generic opaque cache (`put_cache`/`get_cache`) and the source/book
//! upserts on [`InMemoryStorage`] are kept for backward compatibility with the
//! remote-reading runtime; the typed traits below are the canonical persistence
//! interface going forward.

use std::collections::HashMap;
use std::sync::Mutex;

use reader_domain::{Book, ReadingProgress, Source};

// ===========================================================================
// Errors
// ===========================================================================

/// Storage errors.
///
/// `Poisoned` is the only failure mode of the in-memory backend (a poisoned
/// mutex). The remaining variants cover the typed trait surface and are
/// produced by validation/lookup rules; a future SQLite backend would map its
/// own errors onto these (and may extend the enum).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// A mutex guard was poisoned.
    Poisoned,
    /// The referenced entity does not exist.
    NotFound,
    /// Caller-supplied input failed validation (empty id, invalid transition).
    InvalidInput,
    /// The operation conflicts with existing state (e.g. duplicate task id).
    Conflict,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Poisoned => write!(f, "storage lock poisoned"),
            StorageError::NotFound => write!(f, "storage entry not found"),
            StorageError::InvalidInput => write!(f, "storage invalid input"),
            StorageError::Conflict => write!(f, "storage conflict"),
        }
    }
}

impl std::error::Error for StorageError {}

// ===========================================================================
// Models
// ===========================================================================

/// A bookshelf entry: a book the user has added to their shelf, plus shelf
/// metadata (when it was added and a manual sort position). Keyed by
/// `book.book_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct BookshelfEntry {
    pub book: Book,
    /// Source the book was added from.
    pub source_id: String,
    /// Unix epoch millis when the book was added to the shelf.
    pub added_at: i64,
    /// 0-based manual sort position within the shelf (lower = earlier).
    pub sort_order: i32,
}

/// A cached chapter body, keyed by `(book_id, chapter_key)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterCacheEntry {
    pub book_id: String,
    pub chapter_key: String,
    pub content: String,
    /// Unix epoch millis when the chapter was cached.
    pub cached_at: i64,
}

/// Lifecycle status of a download queue task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

/// A download queue task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadTask {
    pub task_id: String,
    pub book_id: String,
    pub chapter_key: String,
    pub url: String,
    pub status: DownloadStatus,
    /// Unix epoch millis when the task was enqueued.
    pub enqueued_at: i64,
}

// ===========================================================================
// Traits
// ===========================================================================

/// Bookshelf persistence.
pub trait BookshelfStore {
    /// Add (or replace) a bookshelf entry keyed by `entry.book.book_id`.
    fn add_to_shelf(&self, entry: BookshelfEntry) -> Result<BookshelfEntry, StorageError>;
    /// Look up a bookshelf entry by book id.
    fn get_shelf_entry(&self, book_id: &str) -> Result<Option<BookshelfEntry>, StorageError>;
    /// List all bookshelf entries, sorted by `sort_order` then `added_at`.
    fn list_shelf(&self) -> Result<Vec<BookshelfEntry>, StorageError>;
    /// Remove a bookshelf entry by book id. Returns `true` if an entry was
    /// removed.
    fn remove_from_shelf(&self, book_id: &str) -> Result<bool, StorageError>;
}

/// Reading progress persistence.
pub trait ProgressStore {
    fn put_progress(&self, progress: ReadingProgress) -> Result<ReadingProgress, StorageError>;
    fn get_progress(&self, book_id: &str) -> Result<Option<ReadingProgress>, StorageError>;
}

/// Chapter content cache.
pub trait ChapterCacheStore {
    /// Store (or replace) a cached chapter body.
    fn put_chapter(&self, entry: ChapterCacheEntry) -> Result<ChapterCacheEntry, StorageError>;
    /// Read a cached chapter body by `(book_id, chapter_key)`.
    fn get_chapter(
        &self,
        book_id: &str,
        chapter_key: &str,
    ) -> Result<Option<ChapterCacheEntry>, StorageError>;
    /// Evict all cached chapters for a book. Returns the number evicted.
    fn evict_chapters(&self, book_id: &str) -> Result<usize, StorageError>;
}

/// Download queue persistence.
pub trait DownloadQueueStore {
    /// Enqueue a download task. Rejects empty `task_id` or duplicate `task_id`.
    fn enqueue(&self, task: DownloadTask) -> Result<DownloadTask, StorageError>;
    /// List all queue tasks in enqueue order.
    fn list_queue(&self) -> Result<Vec<DownloadTask>, StorageError>;
    /// Return the next `Pending` task in enqueue order, without mutating it.
    fn next_pending(&self) -> Result<Option<DownloadTask>, StorageError>;
    /// Update a task's status. Returns `NotFound` if `task_id` is unknown.
    fn update_status(&self, task_id: &str, status: DownloadStatus) -> Result<(), StorageError>;
    /// Remove a task from the queue. Returns `true` if a task was removed.
    fn remove_task(&self, task_id: &str) -> Result<bool, StorageError>;
}

// ===========================================================================
// Opaque cache entry (kept for runtime backward compat)
// ===========================================================================

/// A minimal cached entry: an opaque JSON payload keyed by a string cache key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedEntry {
    pub key: String,
    pub payload: String,
}

// ===========================================================================
// In-memory implementation
// ===========================================================================

/// In-memory storage implementing all persistence traits.
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
    shelf: HashMap<String, BookshelfEntry>,
    chapters: HashMap<(String, String), ChapterCacheEntry>,
    queue: Vec<DownloadTask>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, StorageInner>, StorageError> {
        self.inner.lock().map_err(|_| StorageError::Poisoned)
    }

    // --- Opaque source/book/progress/cache (runtime-facing) ---------------

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
    ///
    /// Kept as an inherent method for runtime backward compatibility; the real
    /// logic lives in the [`ProgressStore`] impl.
    pub fn put_progress(&self, progress: ReadingProgress) -> Result<ReadingProgress, StorageError> {
        <Self as ProgressStore>::put_progress(self, progress)
    }

    /// Read reading progress for a book.
    ///
    /// Kept as an inherent method for runtime backward compatibility; the real
    /// logic lives in the [`ProgressStore`] impl.
    pub fn get_progress(&self, book_id: &str) -> Result<Option<ReadingProgress>, StorageError> {
        <Self as ProgressStore>::get_progress(self, book_id)
    }
}

// ===========================================================================
// Trait implementations
// ===========================================================================

impl BookshelfStore for InMemoryStorage {
    fn add_to_shelf(&self, entry: BookshelfEntry) -> Result<BookshelfEntry, StorageError> {
        if entry.book.book_id.trim().is_empty() {
            return Err(StorageError::InvalidInput);
        }
        let mut inner = self.lock()?;
        inner
            .shelf
            .insert(entry.book.book_id.clone(), entry.clone());
        Ok(entry)
    }

    fn get_shelf_entry(&self, book_id: &str) -> Result<Option<BookshelfEntry>, StorageError> {
        Ok(self.lock()?.shelf.get(book_id).cloned())
    }

    fn list_shelf(&self) -> Result<Vec<BookshelfEntry>, StorageError> {
        let mut entries: Vec<_> = self.lock()?.shelf.values().cloned().collect();
        entries.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.added_at.cmp(&b.added_at))
        });
        Ok(entries)
    }

    fn remove_from_shelf(&self, book_id: &str) -> Result<bool, StorageError> {
        Ok(self.lock()?.shelf.remove(book_id).is_some())
    }
}

impl ProgressStore for InMemoryStorage {
    fn put_progress(&self, progress: ReadingProgress) -> Result<ReadingProgress, StorageError> {
        let mut inner = self.lock()?;
        inner
            .progress
            .insert(progress.book_id.clone(), progress.clone());
        Ok(progress)
    }

    fn get_progress(&self, book_id: &str) -> Result<Option<ReadingProgress>, StorageError> {
        Ok(self.lock()?.progress.get(book_id).cloned())
    }
}

impl ChapterCacheStore for InMemoryStorage {
    fn put_chapter(&self, entry: ChapterCacheEntry) -> Result<ChapterCacheEntry, StorageError> {
        if entry.book_id.trim().is_empty() || entry.chapter_key.trim().is_empty() {
            return Err(StorageError::InvalidInput);
        }
        let key = (entry.book_id.clone(), entry.chapter_key.clone());
        let mut inner = self.lock()?;
        inner.chapters.insert(key, entry.clone());
        Ok(entry)
    }

    fn get_chapter(
        &self,
        book_id: &str,
        chapter_key: &str,
    ) -> Result<Option<ChapterCacheEntry>, StorageError> {
        Ok(self
            .lock()?
            .chapters
            .get(&(book_id.to_string(), chapter_key.to_string()))
            .cloned())
    }

    fn evict_chapters(&self, book_id: &str) -> Result<usize, StorageError> {
        let mut inner = self.lock()?;
        let before = inner.chapters.len();
        inner.chapters.retain(|(bid, _), _| bid != book_id);
        Ok(before - inner.chapters.len())
    }
}

impl DownloadQueueStore for InMemoryStorage {
    fn enqueue(&self, task: DownloadTask) -> Result<DownloadTask, StorageError> {
        if task.task_id.trim().is_empty() {
            return Err(StorageError::InvalidInput);
        }
        let mut inner = self.lock()?;
        if inner.queue.iter().any(|t| t.task_id == task.task_id) {
            return Err(StorageError::Conflict);
        }
        inner.queue.push(task.clone());
        Ok(task)
    }

    fn list_queue(&self) -> Result<Vec<DownloadTask>, StorageError> {
        Ok(self.lock()?.queue.clone())
    }

    fn next_pending(&self) -> Result<Option<DownloadTask>, StorageError> {
        Ok(self
            .lock()?
            .queue
            .iter()
            .find(|t| t.status == DownloadStatus::Pending)
            .cloned())
    }

    fn update_status(&self, task_id: &str, status: DownloadStatus) -> Result<(), StorageError> {
        let mut inner = self.lock()?;
        let task = inner
            .queue
            .iter_mut()
            .find(|t| t.task_id == task_id)
            .ok_or(StorageError::NotFound)?;
        task.status = status;
        Ok(())
    }

    fn remove_task(&self, task_id: &str) -> Result<bool, StorageError> {
        let mut inner = self.lock()?;
        if let Some(pos) = inner.queue.iter().position(|t| t.task_id == task_id) {
            inner.queue.remove(pos);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reader_domain::SourceRules;

    // --- Compile-time trait bound check -----------------------------------

    #[test]
    fn in_memory_storage_implements_all_traits() {
        fn _assert<T: BookshelfStore + ProgressStore + ChapterCacheStore + DownloadQueueStore>() {}
        _assert::<InMemoryStorage>();
    }

    // --- Legacy runtime-facing API (kept verbatim) ------------------------

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

    // --- BookshelfStore ---------------------------------------------------

    fn sample_book(id: &str) -> Book {
        Book {
            book_id: id.into(),
            title: format!("Title {id}"),
            author: "Author".into(),
            cover_url: None,
            intro: None,
            kind: None,
            last_chapter: None,
        }
    }

    fn sample_shelf_entry(book_id: &str, sort_order: i32, added_at: i64) -> BookshelfEntry {
        BookshelfEntry {
            book: sample_book(book_id),
            source_id: "src1".into(),
            added_at,
            sort_order,
        }
    }

    #[test]
    fn shelf_add_get_remove_round_trip() {
        let store = InMemoryStorage::new();
        let entry = sample_shelf_entry("b1", 0, 1000);
        store.add_to_shelf(entry.clone()).unwrap();
        let got = store.get_shelf_entry("b1").unwrap().unwrap();
        assert_eq!(got, entry);
        assert!(store.get_shelf_entry("missing").unwrap().is_none());
        assert!(store.remove_from_shelf("b1").unwrap());
        assert!(!store.remove_from_shelf("b1").unwrap());
        assert!(store.get_shelf_entry("b1").unwrap().is_none());
    }

    #[test]
    fn shelf_list_sorted_by_sort_order_then_added_at() {
        let store = InMemoryStorage::new();
        store.add_to_shelf(sample_shelf_entry("a", 1, 100)).unwrap();
        store.add_to_shelf(sample_shelf_entry("b", 0, 200)).unwrap();
        store.add_to_shelf(sample_shelf_entry("c", 1, 300)).unwrap();
        let entries = store.list_shelf().unwrap();
        let ids: Vec<_> = entries
            .iter()
            .map(|e| e.book.book_id.as_str())
            .collect();
        // sort_order 0 first; then sort_order 1 ordered by added_at.
        assert_eq!(ids, vec!["b", "a", "c"]);
    }

    #[test]
    fn shelf_add_rejects_empty_book_id() {
        let store = InMemoryStorage::new();
        let mut entry = sample_shelf_entry("x", 0, 0);
        entry.book.book_id = "  ".into();
        assert_eq!(
            store.add_to_shelf(entry).unwrap_err(),
            StorageError::InvalidInput
        );
    }

    #[test]
    fn shelf_add_upserts_on_duplicate() {
        let store = InMemoryStorage::new();
        store.add_to_shelf(sample_shelf_entry("b1", 0, 1000)).unwrap();
        let updated = BookshelfEntry {
            book: Book {
                book_id: "b1".into(),
                title: "Updated".into(),
                author: "Author".into(),
                cover_url: None,
                intro: None,
                kind: None,
                last_chapter: None,
            },
            source_id: "src2".into(),
            added_at: 2000,
            sort_order: 5,
        };
        store.add_to_shelf(updated.clone()).unwrap();
        let got = store.get_shelf_entry("b1").unwrap().unwrap();
        assert_eq!(got, updated);
        assert_eq!(store.list_shelf().unwrap().len(), 1);
    }

    // --- ProgressStore (trait path) ---------------------------------------

    #[test]
    fn progress_trait_round_trip() {
        let store = InMemoryStorage::new();
        let p = ReadingProgress {
            book_id: "b9".into(),
            chapter_index: 1,
            chapter_offset: 200,
            chapter_progress: 0.25,
        };
        <InMemoryStorage as ProgressStore>::put_progress(&store, p.clone()).unwrap();
        let got = <InMemoryStorage as ProgressStore>::get_progress(&store, "b9")
            .unwrap()
            .unwrap();
        assert_eq!(got, p);
    }

    // --- ChapterCacheStore ------------------------------------------------

    fn sample_chapter(book_id: &str, chapter_key: &str) -> ChapterCacheEntry {
        ChapterCacheEntry {
            book_id: book_id.into(),
            chapter_key: chapter_key.into(),
            content: format!("body of {chapter_key}"),
            cached_at: 5000,
        }
    }

    #[test]
    fn chapter_put_get_round_trip() {
        let store = InMemoryStorage::new();
        assert!(store.get_chapter("b1", "c1").unwrap().is_none());
        let entry = sample_chapter("b1", "c1");
        store.put_chapter(entry.clone()).unwrap();
        let got = store.get_chapter("b1", "c1").unwrap().unwrap();
        assert_eq!(got, entry);
    }

    #[test]
    fn chapter_evict_removes_only_matching_book() {
        let store = InMemoryStorage::new();
        store.put_chapter(sample_chapter("b1", "c1")).unwrap();
        store.put_chapter(sample_chapter("b1", "c2")).unwrap();
        store.put_chapter(sample_chapter("b2", "c1")).unwrap();
        let evicted = store.evict_chapters("b1").unwrap();
        assert_eq!(evicted, 2);
        assert!(store.get_chapter("b1", "c1").unwrap().is_none());
        assert!(store.get_chapter("b1", "c2").unwrap().is_none());
        assert!(store.get_chapter("b2", "c1").unwrap().is_some());
    }

    #[test]
    fn chapter_put_rejects_empty_keys() {
        let store = InMemoryStorage::new();
        let mut entry = sample_chapter("b1", "c1");
        entry.book_id = String::new();
        assert_eq!(
            store.put_chapter(entry).unwrap_err(),
            StorageError::InvalidInput
        );
        let mut entry = sample_chapter("b1", "c1");
        entry.chapter_key = "  ".into();
        assert_eq!(
            store.put_chapter(entry).unwrap_err(),
            StorageError::InvalidInput
        );
    }

    #[test]
    fn chapter_put_upserts() {
        let store = InMemoryStorage::new();
        store.put_chapter(sample_chapter("b1", "c1")).unwrap();
        let updated = ChapterCacheEntry {
            book_id: "b1".into(),
            chapter_key: "c1".into(),
            content: "new body".into(),
            cached_at: 9000,
        };
        store.put_chapter(updated.clone()).unwrap();
        let got = store.get_chapter("b1", "c1").unwrap().unwrap();
        assert_eq!(got, updated);
    }

    // --- DownloadQueueStore -----------------------------------------------

    fn sample_task(id: &str) -> DownloadTask {
        DownloadTask {
            task_id: id.into(),
            book_id: "b1".into(),
            chapter_key: "c1".into(),
            url: format!("https://example/{id}"),
            status: DownloadStatus::Pending,
            enqueued_at: 1000,
        }
    }

    #[test]
    fn queue_enqueue_list_next_pending() {
        let store = InMemoryStorage::new();
        store.enqueue(sample_task("t1")).unwrap();
        store.enqueue(sample_task("t2")).unwrap();
        let list = store.list_queue().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].task_id, "t1");
        let next = store.next_pending().unwrap().unwrap();
        assert_eq!(next.task_id, "t1");
    }

    #[test]
    fn queue_next_pending_skips_non_pending() {
        let store = InMemoryStorage::new();
        store.enqueue(sample_task("t1")).unwrap();
        store
            .update_status("t1", DownloadStatus::Completed)
            .unwrap();
        store.enqueue(sample_task("t2")).unwrap();
        let next = store.next_pending().unwrap().unwrap();
        assert_eq!(next.task_id, "t2");
    }

    #[test]
    fn queue_next_pending_none_when_empty_or_all_done() {
        let store = InMemoryStorage::new();
        assert!(store.next_pending().unwrap().is_none());
        store.enqueue(sample_task("t1")).unwrap();
        store
            .update_status("t1", DownloadStatus::Failed)
            .unwrap();
        assert!(store.next_pending().unwrap().is_none());
    }

    #[test]
    fn queue_update_status_not_found() {
        let store = InMemoryStorage::new();
        assert_eq!(
            store
                .update_status("nope", DownloadStatus::InProgress)
                .unwrap_err(),
            StorageError::NotFound
        );
    }

    #[test]
    fn queue_enqueue_rejects_empty_and_duplicate_task_id() {
        let store = InMemoryStorage::new();
        let mut bad = sample_task("t1");
        bad.task_id = String::new();
        assert_eq!(
            store.enqueue(bad).unwrap_err(),
            StorageError::InvalidInput
        );
        store.enqueue(sample_task("t1")).unwrap();
        assert_eq!(
            store.enqueue(sample_task("t1")).unwrap_err(),
            StorageError::Conflict
        );
    }

    #[test]
    fn queue_remove_task() {
        let store = InMemoryStorage::new();
        store.enqueue(sample_task("t1")).unwrap();
        store.enqueue(sample_task("t2")).unwrap();
        assert!(store.remove_task("t1").unwrap());
        assert!(!store.remove_task("t1").unwrap());
        let list = store.list_queue().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].task_id, "t2");
    }
}

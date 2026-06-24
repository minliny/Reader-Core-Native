//! Reader-Core storage — SQLite schema, migrations, cache, progress, download queue.
//!
//! V1 ships an in-memory implementation only. The real SQLite-backed store is
//! deferred to a later phase; the trait surface here is what the runtime
//! vertical commands depend on, so swapping the backend later is localized to
//! this crate.

use std::collections::HashMap;
use std::sync::Mutex;

use reader_domain::{Book, ReadingProgress, Source};
use serde::{Deserialize, Serialize};

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
}

/// A minimal cached entry: an opaque JSON payload keyed by a string cache key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedEntry {
    pub key: String,
    pub payload: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShelfKey {
    source_id: String,
    book_id: String,
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
    fn remove_from_shelf(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<(), StorageError>;

    /// Look up a single shelf entry by composite key.
    fn get_shelf_entry(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Option<BookshelfEntry>, StorageError>;

    /// List all shelf entries, sorted by `sort_index` ascending then
    /// `added_at` descending (most recently added first within the same index).
    fn list_shelf(&self) -> Result<Vec<BookshelfEntry>, StorageError>;

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

    /// Number of entries on the shelf.
    fn shelf_count(&self) -> Result<usize, StorageError>;
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

fn validate_shelf_key(source_id: &str, book_id: &str) -> Result<(), StorageError> {
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

fn sort_shelf(entries: &mut Vec<BookshelfEntry>) {
    // sort_index ascending; ties broken by added_at descending (newer first).
    entries.sort_by(|a, b| {
        a.sort_index
            .cmp(&b.sort_index)
            .then_with(|| b.added_at.cmp(&a.added_at))
    });
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

    fn remove_from_shelf(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<(), StorageError> {
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

    fn list_shelf_by_group(&self, group: &str) -> Result<Vec<BookshelfEntry>, StorageError> {
        let inner = self.lock()?;
        let mut entries: Vec<BookshelfEntry> = inner
            .shelf
            .values()
            .filter(|e| e.group.as_deref() == Some(group))
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

    fn shelf_count(&self) -> Result<usize, StorageError> {
        Ok(self.lock()?.shelf.len())
    }
}

/// Storage errors. V1 is in-memory so the only realistic failure is a poisoned
/// lock; the variant exists so the runtime can surface a structured `INTERNAL`
/// error instead of panicking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    Poisoned,
    /// A key field (source_id or book_id) was empty or invalid.
    InvalidKey { field: String },
    /// An entry referenced by the operation does not exist.
    NotFound { source_id: String, book_id: String },
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Poisoned => write!(f, "storage lock poisoned"),
            StorageError::InvalidKey { field } => write!(f, "invalid key field: {field}"),
            StorageError::NotFound { source_id, book_id } => {
                write!(f, "shelf entry not found: source={source_id} book={book_id}")
            }
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
        store.add_to_shelf(shelf_entry("s1", "b1", "Dune", 1000)).unwrap();
        store.remove_from_shelf("s1", "b1").unwrap();
        assert!(store.get_shelf_entry("s1", "b1").unwrap().is_none());
        // Removing again is not an error.
        store.remove_from_shelf("s1", "b1").unwrap();
        assert_eq!(store.shelf_count().unwrap(), 0);
    }

    #[test]
    fn shelf_get_missing_returns_none() {
        let store = InMemoryStorage::new();
        store.add_to_shelf(shelf_entry("s1", "b1", "Dune", 1000)).unwrap();
        assert!(store.get_shelf_entry("s1", "missing").unwrap().is_none());
        assert!(store.get_shelf_entry("missing", "b1").unwrap().is_none());
    }

    #[test]
    fn shelf_cross_source_same_book_id_no_collision() {
        let store = InMemoryStorage::new();
        store.add_to_shelf(shelf_entry("s1", "b1", "From S1", 1000)).unwrap();
        store.add_to_shelf(shelf_entry("s2", "b1", "From S2", 2000)).unwrap();
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
        store.add_to_shelf(shelf_entry("s1", "b1", "A", 1000)).unwrap();
        // sort_index 0, added_at 2000 (newer) — should come before A within index 0
        store.add_to_shelf(shelf_entry("s1", "b2", "B", 2000)).unwrap();
        // sort_index 1 — should come after both index-0 entries
        let mut c = shelf_entry("s1", "b3", "C", 500);
        c.sort_index = 1;
        store.add_to_shelf(c).unwrap();

        let list = store.list_shelf().unwrap();
        let titles: Vec<&str> = list.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, vec!["B", "A", "C"], "sorted by index asc, added_at desc");
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
    fn shelf_update_last_read_sets_timestamp() {
        let store = InMemoryStorage::new();
        store.add_to_shelf(shelf_entry("s1", "b1", "Dune", 1000)).unwrap();
        assert!(store.get_shelf_entry("s1", "b1").unwrap().unwrap().last_read_at.is_none());
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
        assert_eq!(err, StorageError::InvalidKey { field: "source_id".into() });

        // Whitespace-only also rejected.
        entry.source_id = "   ".into();
        let err = store.add_to_shelf(entry).unwrap_err();
        assert_eq!(err, StorageError::InvalidKey { field: "source_id".into() });
    }

    #[test]
    fn shelf_rejects_empty_book_id() {
        let store = InMemoryStorage::new();
        let entry = shelf_entry("s1", "", "Dune", 1000);
        let err = store.add_to_shelf(entry).unwrap_err();
        assert_eq!(err, StorageError::InvalidKey { field: "book_id".into() });
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
}

//! Reader-Core storage — SQLite schema, migrations, cache, progress, download queue.
//!
//! V1 ships an in-memory implementation only. The real SQLite-backed store is
//! deferred to a later phase; the trait surface here is what the runtime
//! vertical commands depend on, so swapping the backend later is localized to
//! this crate.

use std::collections::HashMap;
use std::sync::Mutex;

use reader_domain::{Book, ReadingProgress, Source};

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
}

/// A minimal cached entry: an opaque JSON payload keyed by a string cache key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedEntry {
    pub key: String,
    pub payload: String,
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

/// Storage errors. V1 is in-memory so the only realistic failure is a poisoned
/// lock; the variant exists so the runtime can surface a structured `INTERNAL`
/// error instead of panicking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    Poisoned,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Poisoned => write!(f, "storage lock poisoned"),
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
}

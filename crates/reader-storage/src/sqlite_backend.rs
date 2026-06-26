//! SQLite-backed persistent storage for `reader-storage`.
//!
//! Implements the same five traits as [`crate::InMemoryStorage`] (`BookshelfStore`,
//! `ChapterCacheStore`, `ReadingProgressStore`, `ChapterDownloadQueueStore`,
//! `StorageSnapshotStore`) plus the inherent source/book/cache/progress helpers,
//! but backed by a real on-disk SQLite database so reader state survives process
//! restarts. This closes the P0 persistence gap noted in `FEATURE_MATRIX.md`.
//!
//! ## Schema versioning
//!
//! The schema is tracked via SQLite's `PRAGMA user_version`. The migrator in
//! [`SqliteStorage::migrate`] runs forward-only migrations from any older
//! `user_version` up to [`SQLITE_SCHEMA_VERSION`]. There is no downgrading: a
//! database with a newer `user_version` than this binary understands is rejected
//! as [`StorageError::InvalidSnapshot`] rather than silently truncated, matching
//! the JSON snapshot migrator's policy.
//!
//! The JSON [`crate::StorageSnapshot`] migrator (`migrate_storage_snapshot`,
//! schema_version 0 → 1) is orthogonal to this SQL migration: the JSON snapshot
//! is the transport-neutral backup/restore shape, the SQL `user_version` is the
//! on-disk schema version. They are intentionally separate numbering spaces.
//!
//! ## Layout
//!
//! `sources` / `books` / `cache` store their whole value as a single JSON TEXT
//! column (lossless, schema-stable, matches `InMemoryStorage` semantics where
//! these are opaque keyed maps). The five reader-domain tables
//! (`bookshelf`, `chapter_cache`, `reading_progress`, `reading_progress_history`,
//! `chapter_download_queue`) use real columns because the trait surface filters,
//! sorts, and plans against those fields directly.

use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use reader_domain::{Book, ReadingProgress, Source};

use crate::{
    chapter_cache_coverage_from_entries, chapter_cache_stats_from_entries, paginate_shelf,
    sort_shelf, sort_shelf_query, sort_storage_snapshot, validate_book_key,
    validate_chapter_anchor, validate_chapter_count, validate_chapter_download_task,
    validate_prefetch_limit, validate_reading_progress, validate_shelf_key, validate_source_id,
    BookshelfEntry, BookshelfQuery, BookshelfStore, CachedEntry, ChapterCacheCoverage,
    ChapterCacheEntry, ChapterCacheEvictionReport, ChapterCachePrefetchPlan,
    ChapterCacheRetentionPolicy, ChapterCacheStats, ChapterCacheStore, ChapterDownloadQueueStore,
    ChapterDownloadStatus, ChapterDownloadTask, ReadingProgressEntry, ReadingProgressStore,
    StorageError, StorageSnapshot, StorageSnapshotStore, STORAGE_SNAPSHOT_SCHEMA_VERSION,
};

/// Current on-disk SQLite schema version. Mirrors `PRAGMA user_version`.
///
/// Bump this and register a forward migration in [`migrate`] whenever the DDL
/// in [`SCHEMA_V1_DDL`] (or its successors) changes. The JSON snapshot
/// `schemaVersion` ([`STORAGE_SNAPSHOT_SCHEMA_VERSION`]) is a separate
/// numbering space and must NOT be coupled to this constant.
pub const SQLITE_SCHEMA_VERSION: u32 = 1;

/// DDL for schema v1. Executed verbatim when a database is created at v0.
///
/// Design notes:
/// - No foreign keys. The five reader-domain tables are independent composite-keyed
///   rows, mirroring `InMemoryStorage`'s `HashMap` semantics where a shelf entry
///   can exist without a matching `books` row, and a chapter cache entry can
///   exist without a shelf entry. Enforcing FKs here would diverge from the
///   in-memory backend's permissive behavior.
/// - `sources` / `books` store the whole value as JSON TEXT. Their struct shapes
///   are dynamic (Legado `extra` bag, optional fields) and rarely queried by
///   sub-field, so a single JSON column is lossless and schema-stable.
/// - `reading_progress_history.seq` is a per-book monotonic counter so history
///   preserves insertion order across restarts (the in-memory backend uses a
///   `Vec` whose order is the append order).
pub const SCHEMA_V1_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS sources (
    source_id    TEXT PRIMARY KEY,
    source_json  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS books (
    book_id   TEXT PRIMARY KEY,
    book_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cache (
    key     TEXT PRIMARY KEY,
    payload TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS bookshelf (
    source_id     TEXT NOT NULL,
    book_id       TEXT NOT NULL,
    title         TEXT NOT NULL DEFAULT '',
    author        TEXT NOT NULL DEFAULT '',
    cover_url     TEXT,
    intro         TEXT,
    kind          TEXT,
    last_chapter  TEXT,
    added_at      INTEGER NOT NULL,
    last_read_at  INTEGER,
    "group"       TEXT,
    sort_index    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (source_id, book_id)
);
CREATE INDEX IF NOT EXISTS idx_bookshelf_group ON bookshelf("group");
CREATE INDEX IF NOT EXISTS idx_bookshelf_sort ON bookshelf(sort_index, added_at);

CREATE TABLE IF NOT EXISTS chapter_cache (
    source_id      TEXT NOT NULL,
    book_id        TEXT NOT NULL,
    chapter_index  INTEGER NOT NULL,
    title          TEXT NOT NULL DEFAULT '',
    url            TEXT NOT NULL DEFAULT '',
    content        TEXT NOT NULL DEFAULT '',
    cached_at      INTEGER NOT NULL,
    revision       TEXT,
    PRIMARY KEY (source_id, book_id, chapter_index)
);
CREATE INDEX IF NOT EXISTS idx_chapter_cache_book ON chapter_cache(source_id, book_id);

CREATE TABLE IF NOT EXISTS reading_progress (
    source_id          TEXT NOT NULL,
    book_id            TEXT NOT NULL,
    chapter_index      INTEGER NOT NULL DEFAULT 0,
    chapter_offset     INTEGER NOT NULL DEFAULT 0,
    chapter_progress   REAL NOT NULL DEFAULT 0.0,
    updated_at         INTEGER NOT NULL,
    device_id          TEXT,
    PRIMARY KEY (source_id, book_id)
);
CREATE INDEX IF NOT EXISTS idx_reading_progress_source ON reading_progress(source_id, updated_at);

CREATE TABLE IF NOT EXISTS reading_progress_history (
    source_id          TEXT NOT NULL,
    book_id            TEXT NOT NULL,
    seq                INTEGER NOT NULL,
    chapter_index      INTEGER NOT NULL DEFAULT 0,
    chapter_offset     INTEGER NOT NULL DEFAULT 0,
    chapter_progress   REAL NOT NULL DEFAULT 0.0,
    updated_at         INTEGER NOT NULL,
    device_id          TEXT,
    PRIMARY KEY (source_id, book_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_progress_history_book ON reading_progress_history(source_id, book_id, seq);

CREATE TABLE IF NOT EXISTS chapter_download_queue (
    source_id      TEXT NOT NULL,
    book_id        TEXT NOT NULL,
    chapter_index  INTEGER NOT NULL,
    title          TEXT NOT NULL DEFAULT '',
    url            TEXT NOT NULL DEFAULT '',
    priority       INTEGER NOT NULL DEFAULT 0,
    status         TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    attempts       INTEGER NOT NULL DEFAULT 0,
    max_attempts   INTEGER NOT NULL,
    last_error     TEXT,
    PRIMARY KEY (source_id, book_id, chapter_index)
);
CREATE INDEX IF NOT EXISTS idx_download_claim ON chapter_download_queue(status, priority, updated_at, created_at);
"#;

/// SQLite-backed reader storage. Holds a single connection guarded by a `Mutex`,
/// mirroring the `InMemoryStorage` concurrency model.
pub struct SqliteStorage {
    conn: Mutex<Connection>,
}

impl SqliteStorage {
    /// Open or create a database at `path`, run migrations, and return a ready
    /// store. The parent directory must exist.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StorageError> {
        let conn = Connection::open(path).map_err(rusqlite_error)?;
        Self::init_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open a transient in-memory database (`:memory:`). Useful for tests; the
    /// database ceases to exist when the last connection is dropped. Despite
    /// being in-memory, this exercises the real SQLite schema, migrations, and
    /// SQL execution paths, so it is a meaningful persistence test.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory().map_err(rusqlite_error)?;
        Self::init_connection(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_connection(conn: &Connection) -> Result<(), StorageError> {
        // WAL keeps readers unblocked while a writer holds the lock; for a
        // single-connection store this mostly matters for future multi-conn use.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(rusqlite_error)?;
        Self::migrate(conn)?;
        Ok(())
    }

    /// Run forward-only migrations from the database's current `user_version`
    /// up to [`SQLITE_SCHEMA_VERSION`]. A database at v0 (fresh or pre-schema)
    /// receives the full v1 DDL. A database at a version newer than this binary
    /// understands is rejected rather than silently downgraded.
    pub fn migrate(conn: &Connection) -> Result<(), StorageError> {
        let current: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(rusqlite_error)?;

        if current > SQLITE_SCHEMA_VERSION {
            return Err(StorageError::InvalidSnapshot {
                field: format!(
                    "sqlite user_version={current} newer than supported={SQLITE_SCHEMA_VERSION}"
                ),
            });
        }

        if current < 1 {
            conn.execute_batch(SCHEMA_V1_DDL).map_err(rusqlite_error)?;
        }
        // Future migrations: `if current < 2 { conn.execute_batch(SCHEMA_V2_DDL)?; }`

        if current != SQLITE_SCHEMA_VERSION {
            conn.pragma_update(None, "user_version", SQLITE_SCHEMA_VERSION)
                .map_err(rusqlite_error)?;
        }
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StorageError> {
        self.conn.lock().map_err(|_| StorageError::Poisoned)
    }

    // ----- inherent source/book/cache/progress helpers (mirror InMemoryStorage) -----

    /// Import (upsert) a source definition. Returns the stored source.
    pub fn put_source(&self, source: Source) -> Result<Source, StorageError> {
        validate_source_id(&source.source_id)?;
        let json = serde_json::to_string(&source).map_err(|_| StorageError::InvalidSnapshot {
            field: "source_json".into(),
        })?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO sources (source_id, source_json) VALUES (?1, ?2)",
            params![source.source_id, json],
        )
        .map_err(rusqlite_error)?;
        Ok(source)
    }

    /// Look up a source by id.
    pub fn get_source(&self, source_id: &str) -> Result<Option<Source>, StorageError> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row(
                "SELECT source_json FROM sources WHERE source_id = ?1",
                params![source_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(rusqlite_error)?;
        match json {
            None => Ok(None),
            Some(s) => {
                serde_json::from_str(&s)
                    .map(Some)
                    .map_err(|_| StorageError::InvalidSnapshot {
                        field: "source_json".into(),
                    })
            }
        }
    }

    /// Upsert a cached book record keyed by book id.
    pub fn put_book(&self, book: Book) -> Result<Book, StorageError> {
        if book.book_id.trim().is_empty() {
            return Err(StorageError::InvalidKey {
                field: "book_id".into(),
            });
        }
        let json = serde_json::to_string(&book).map_err(|_| StorageError::InvalidSnapshot {
            field: "book_json".into(),
        })?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO books (book_id, book_json) VALUES (?1, ?2)",
            params![book.book_id, json],
        )
        .map_err(rusqlite_error)?;
        Ok(book)
    }

    pub fn get_book(&self, book_id: &str) -> Result<Option<Book>, StorageError> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row(
                "SELECT book_json FROM books WHERE book_id = ?1",
                params![book_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(rusqlite_error)?;
        match json {
            None => Ok(None),
            Some(s) => {
                serde_json::from_str(&s)
                    .map(Some)
                    .map_err(|_| StorageError::InvalidSnapshot {
                        field: "book_json".into(),
                    })
            }
        }
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
        if entry.key.trim().is_empty() {
            return Err(StorageError::InvalidKey {
                field: "key".into(),
            });
        }
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO cache (key, payload) VALUES (?1, ?2)",
            params![entry.key, entry.payload],
        )
        .map_err(rusqlite_error)?;
        Ok(entry)
    }

    /// Read a cache entry by key.
    pub fn get_cache(&self, key: &str) -> Result<Option<CachedEntry>, StorageError> {
        let conn = self.lock()?;
        let payload: Option<String> = conn
            .query_row(
                "SELECT payload FROM cache WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(rusqlite_error)?;
        Ok(payload.map(|p| CachedEntry {
            key: key.to_string(),
            payload: p,
        }))
    }

    /// Upsert legacy reading progress for a book (the `ReadingProgress` domain
    /// model keyed only by `book_id`). The composite-keyed `ReadingProgressEntry`
    /// is the preferred surface; this mirrors `InMemoryStorage::put_progress`
    /// for callers that still use the legacy model.
    pub fn put_progress(&self, progress: ReadingProgress) -> Result<ReadingProgress, StorageError> {
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
        let conn = self.lock()?;
        // Legacy progress is not exposed via a trait; store as a synthetic
        // reading_progress row under source_id = "legacy" so it round-trips
        // through export/import but does not collide with composite-keyed entries.
        conn.execute(
            "INSERT OR REPLACE INTO reading_progress \
             (source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0, NULL)",
            params![
                "legacy",
                progress.book_id,
                progress.chapter_index,
                progress.chapter_offset as i64,
                progress.chapter_progress,
            ],
        )
        .map_err(rusqlite_error)?;
        Ok(progress)
    }

    /// Read legacy reading progress for a book.
    pub fn get_progress(&self, book_id: &str) -> Result<Option<ReadingProgress>, StorageError> {
        let conn = self.lock()?;
        let row: Option<(u32, i64, f64)> = conn
            .query_row(
                "SELECT chapter_index, chapter_offset, chapter_progress \
                 FROM reading_progress WHERE source_id = 'legacy' AND book_id = ?1",
                params![book_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(rusqlite_error)?;
        Ok(row.map(|(idx, off, frac)| ReadingProgress {
            book_id: book_id.to_string(),
            chapter_index: idx,
            chapter_offset: off as u64,
            chapter_progress: frac,
        }))
    }

    /// Canonical SHA-256 hash of the current storage state, computed over the
    /// deterministically-sorted JSON [`StorageSnapshot`] export. This is the S5
    /// exit-condition primitive: two stores with the same reader data produce
    /// the same 64-hex-char hash, regardless of insert order or row id.
    ///
    /// The snapshot is sorted via [`sort_storage_snapshot`] before hashing, so
    /// the hash is stable across restarts and across `InMemoryStorage` vs
    /// `SqliteStorage` backends (both produce the same canonical snapshot bytes).
    pub fn canonical_snapshot_hash(&self, exported_at: i64) -> Result<String, StorageError> {
        use sha2::{Digest, Sha256};
        let snapshot = self.export_snapshot(exported_at)?;
        let bytes = serde_json::to_vec(&snapshot).map_err(|_| StorageError::InvalidSnapshot {
            field: "snapshot_serialize".into(),
        })?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Current on-disk `user_version`. Exposed for tests and migration tooling.
    pub fn user_version(&self) -> Result<u32, StorageError> {
        let conn = self.lock()?;
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(rusqlite_error)
    }
}

// ===== status enum DB-string mapping =====

fn download_status_to_db(status: &ChapterDownloadStatus) -> &'static str {
    match status {
        ChapterDownloadStatus::Pending => "pending",
        ChapterDownloadStatus::InProgress => "in_progress",
        ChapterDownloadStatus::Completed => "completed",
        ChapterDownloadStatus::Failed => "failed",
        ChapterDownloadStatus::Cancelled => "cancelled",
    }
}

fn download_status_from_db(s: &str) -> Result<ChapterDownloadStatus, StorageError> {
    match s {
        "pending" => Ok(ChapterDownloadStatus::Pending),
        "in_progress" => Ok(ChapterDownloadStatus::InProgress),
        "completed" => Ok(ChapterDownloadStatus::Completed),
        "failed" => Ok(ChapterDownloadStatus::Failed),
        "cancelled" => Ok(ChapterDownloadStatus::Cancelled),
        _ => Err(StorageError::InvalidDownloadTask {
            field: "status".into(),
        }),
    }
}

// ===== row decoders =====

fn row_to_bookshelf_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<BookshelfEntry> {
    Ok(BookshelfEntry {
        source_id: row.get("source_id")?,
        book_id: row.get("book_id")?,
        title: row.get("title")?,
        author: row.get("author")?,
        cover_url: row.get("cover_url")?,
        intro: row.get("intro")?,
        kind: row.get("kind")?,
        last_chapter: row.get("last_chapter")?,
        added_at: row.get("added_at")?,
        last_read_at: row.get("last_read_at")?,
        group: row.get("group")?,
        sort_index: row.get("sort_index")?,
    })
}

fn row_to_chapter_cache_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChapterCacheEntry> {
    Ok(ChapterCacheEntry {
        source_id: row.get("source_id")?,
        book_id: row.get("book_id")?,
        chapter_index: row.get::<_, i64>("chapter_index")? as u32,
        title: row.get("title")?,
        url: row.get("url")?,
        content: row.get("content")?,
        cached_at: row.get("cached_at")?,
        revision: row.get("revision")?,
    })
}

fn row_to_reading_progress_entry(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ReadingProgressEntry> {
    Ok(ReadingProgressEntry {
        source_id: row.get("source_id")?,
        book_id: row.get("book_id")?,
        chapter_index: row.get::<_, i64>("chapter_index")? as u32,
        chapter_offset: row.get::<_, i64>("chapter_offset")? as u64,
        chapter_progress: row.get("chapter_progress")?,
        updated_at: row.get("updated_at")?,
        device_id: row.get("device_id")?,
    })
}

fn row_to_chapter_download_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChapterDownloadTask> {
    let status_str: String = row.get("status")?;
    let status = download_status_from_db(&status_str).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
    })?;
    Ok(ChapterDownloadTask {
        source_id: row.get("source_id")?,
        book_id: row.get("book_id")?,
        chapter_index: row.get::<_, i64>("chapter_index")? as u32,
        title: row.get("title")?,
        url: row.get("url")?,
        priority: row.get("priority")?,
        status,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        attempts: row.get::<_, i64>("attempts")? as u32,
        max_attempts: row.get::<_, i64>("max_attempts")? as u32,
        last_error: row.get("last_error")?,
    })
}

fn rusqlite_error(err: rusqlite::Error) -> StorageError {
    // V1: collapse rusqlite's error variants into a single storage-internal
    // bucket. The error Display is preserved via the InvalidSnapshot field so
    // callers can still see what went wrong, but we do not leak rusqlite types
    // through the public StorageError surface.
    StorageError::InvalidSnapshot {
        field: format!("sqlite: {err}"),
    }
}

// ===== BookshelfStore =====

impl BookshelfStore for SqliteStorage {
    fn add_to_shelf(&self, entry: BookshelfEntry) -> Result<BookshelfEntry, StorageError> {
        validate_shelf_key(&entry.source_id, &entry.book_id)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(rusqlite_error)?;
        // Preserve original added_at on upsert.
        let existing_added_at: Option<i64> = tx
            .query_row(
                "SELECT added_at FROM bookshelf WHERE source_id = ?1 AND book_id = ?2",
                params![entry.source_id, entry.book_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(rusqlite_error)?;
        let stored = BookshelfEntry {
            added_at: existing_added_at.unwrap_or(entry.added_at),
            ..entry.clone()
        };
        tx.execute(
            "INSERT OR REPLACE INTO bookshelf \
             (source_id, book_id, title, author, cover_url, intro, kind, last_chapter, \
              added_at, last_read_at, \"group\", sort_index) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                stored.source_id,
                stored.book_id,
                stored.title,
                stored.author,
                stored.cover_url,
                stored.intro,
                stored.kind,
                stored.last_chapter,
                stored.added_at,
                stored.last_read_at,
                stored.group,
                stored.sort_index,
            ],
        )
        .map_err(rusqlite_error)?;
        tx.commit().map_err(rusqlite_error)?;
        Ok(stored)
    }

    fn remove_from_shelf(&self, source_id: &str, book_id: &str) -> Result<(), StorageError> {
        validate_shelf_key(source_id, book_id)?;
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM bookshelf WHERE source_id = ?1 AND book_id = ?2",
            params![source_id, book_id],
        )
        .map_err(rusqlite_error)?;
        Ok(())
    }

    fn get_shelf_entry(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Option<BookshelfEntry>, StorageError> {
        validate_shelf_key(source_id, book_id)?;
        let conn = self.lock()?;
        conn.query_row(
            "SELECT source_id, book_id, title, author, cover_url, intro, kind, last_chapter, \
             added_at, last_read_at, \"group\", sort_index \
             FROM bookshelf WHERE source_id = ?1 AND book_id = ?2",
            params![source_id, book_id],
            row_to_bookshelf_entry,
        )
        .optional()
        .map_err(rusqlite_error)
    }

    fn list_shelf(&self) -> Result<Vec<BookshelfEntry>, StorageError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, title, author, cover_url, intro, kind, last_chapter, \
                 added_at, last_read_at, \"group\", sort_index FROM bookshelf",
            )
            .map_err(rusqlite_error)?;
        let mut entries = stmt
            .query_map([], row_to_bookshelf_entry)
            .map_err(rusqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(rusqlite_error)?;
        sort_shelf(&mut entries);
        Ok(entries)
    }

    fn query_shelf(&self, query: BookshelfQuery) -> Result<Vec<BookshelfEntry>, StorageError> {
        let source_id = crate::normalize_required_filter(query.source_id, "source_id")?;
        let group = crate::normalize_group(query.group)?;
        let keyword = crate::normalize_keyword(query.keyword);
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, title, author, cover_url, intro, kind, last_chapter, \
                 added_at, last_read_at, \"group\", sort_index FROM bookshelf",
            )
            .map_err(rusqlite_error)?;
        let mut entries = stmt
            .query_map([], row_to_bookshelf_entry)
            .map_err(rusqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(rusqlite_error)?;

        // has_reading_progress filter requires a reading_progress row existence
        // check — cheaper to filter in Rust after loading than to issue a
        // correlated subquery per row, given shelf sizes are modest.
        let progress_rows: Vec<(String, String)> = conn
            .prepare("SELECT source_id, book_id FROM reading_progress")
            .map_err(rusqlite_error)?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(rusqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(rusqlite_error)?;
        let progress_set: std::collections::HashSet<(String, String)> =
            progress_rows.into_iter().collect();

        entries.retain(|entry| {
            source_id
                .as_deref()
                .map(|s| entry.source_id == s)
                .unwrap_or(true)
        });
        entries.retain(|entry| {
            group
                .as_deref()
                .map(|g| entry.group.as_deref() == Some(g))
                .unwrap_or(true)
        });
        if let Some(kw) = keyword.as_deref() {
            entries.retain(|entry| crate::entry_matches_keyword(entry, kw));
        }
        if let Some(expected) = query.has_reading_progress {
            entries.retain(|entry| {
                let has = progress_set.contains(&(entry.source_id.clone(), entry.book_id.clone()));
                has == expected
            });
        }
        sort_shelf_query(&mut entries, query.sort_by, query.sort_direction);
        Ok(paginate_shelf(entries, query.offset, query.limit))
    }

    fn list_shelf_by_group(&self, group: &str) -> Result<Vec<BookshelfEntry>, StorageError> {
        let group = crate::normalize_group(Some(group.to_string()))?
            .expect("normalize_group returns Some for Some input");
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, title, author, cover_url, intro, kind, last_chapter, \
                 added_at, last_read_at, \"group\", sort_index FROM bookshelf \
                 WHERE \"group\" = ?1",
            )
            .map_err(rusqlite_error)?;
        let mut entries = stmt
            .query_map(params![group], row_to_bookshelf_entry)
            .map_err(rusqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(rusqlite_error)?;
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
        let conn = self.lock()?;
        let affected = conn
            .execute(
                "UPDATE bookshelf SET last_read_at = ?1 WHERE source_id = ?2 AND book_id = ?3",
                params![timestamp, source_id, book_id],
            )
            .map_err(rusqlite_error)?;
        if affected == 0 {
            return Err(StorageError::NotFound {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
            });
        }
        Ok(())
    }

    fn move_shelf_entry(
        &self,
        source_id: &str,
        book_id: &str,
        group: Option<String>,
        sort_index: i32,
    ) -> Result<BookshelfEntry, StorageError> {
        validate_shelf_key(source_id, book_id)?;
        let conn = self.lock()?;
        let affected = conn
            .execute(
                "UPDATE bookshelf SET \"group\" = ?1, sort_index = ?2 \
                 WHERE source_id = ?3 AND book_id = ?4",
                params![group, sort_index, source_id, book_id],
            )
            .map_err(rusqlite_error)?;
        if affected == 0 {
            return Err(StorageError::NotFound {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
            });
        }
        conn.query_row(
            "SELECT source_id, book_id, title, author, cover_url, intro, kind, last_chapter, \
             added_at, last_read_at, \"group\", sort_index \
             FROM bookshelf WHERE source_id = ?1 AND book_id = ?2",
            params![source_id, book_id],
            row_to_bookshelf_entry,
        )
        .map_err(rusqlite_error)
    }

    fn shelf_count(&self) -> Result<usize, StorageError> {
        let conn = self.lock()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM bookshelf", [], |row| row.get(0))
            .map_err(rusqlite_error)?;
        Ok(count as usize)
    }
}

// ===== ChapterCacheStore =====

impl ChapterCacheStore for SqliteStorage {
    fn put_chapter_cache(
        &self,
        entry: ChapterCacheEntry,
    ) -> Result<ChapterCacheEntry, StorageError> {
        validate_book_key(&entry.source_id, &entry.book_id)?;
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO chapter_cache \
             (source_id, book_id, chapter_index, title, url, content, cached_at, revision) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.source_id,
                entry.book_id,
                entry.chapter_index,
                entry.title,
                entry.url,
                entry.content,
                entry.cached_at,
                entry.revision,
            ],
        )
        .map_err(rusqlite_error)?;
        Ok(entry)
    }

    fn get_chapter_cache(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<Option<ChapterCacheEntry>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let conn = self.lock()?;
        conn.query_row(
            "SELECT source_id, book_id, chapter_index, title, url, content, cached_at, revision \
             FROM chapter_cache WHERE source_id = ?1 AND book_id = ?2 AND chapter_index = ?3",
            params![source_id, book_id, chapter_index],
            row_to_chapter_cache_entry,
        )
        .optional()
        .map_err(rusqlite_error)
    }

    fn remove_chapter_cache(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<(), StorageError> {
        validate_book_key(source_id, book_id)?;
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM chapter_cache \
             WHERE source_id = ?1 AND book_id = ?2 AND chapter_index = ?3",
            params![source_id, book_id, chapter_index],
        )
        .map_err(rusqlite_error)?;
        Ok(())
    }

    fn list_chapter_cache(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Vec<ChapterCacheEntry>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, title, url, content, cached_at, revision \
                 FROM chapter_cache WHERE source_id = ?1 AND book_id = ?2 ORDER BY chapter_index ASC",
            )
            .map_err(rusqlite_error)?;
        let rows = stmt
            .query_map(params![source_id, book_id], row_to_chapter_cache_entry)
            .map_err(rusqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(rusqlite_error)
    }

    fn clear_chapter_cache(&self, source_id: &str, book_id: &str) -> Result<usize, StorageError> {
        validate_book_key(source_id, book_id)?;
        let conn = self.lock()?;
        let removed = conn
            .execute(
                "DELETE FROM chapter_cache WHERE source_id = ?1 AND book_id = ?2",
                params![source_id, book_id],
            )
            .map_err(rusqlite_error)?;
        Ok(removed)
    }

    fn chapter_cache_coverage(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_count: u32,
    ) -> Result<ChapterCacheCoverage, StorageError> {
        validate_book_key(source_id, book_id)?;
        validate_chapter_count(chapter_count)?;
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, title, url, content, cached_at, revision \
                 FROM chapter_cache WHERE source_id = ?1 AND book_id = ?2",
            )
            .map_err(rusqlite_error)?;
        let entries = stmt
            .query_map(params![source_id, book_id], row_to_chapter_cache_entry)
            .map_err(rusqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(rusqlite_error)?;
        Ok(chapter_cache_coverage_from_entries(
            source_id,
            book_id,
            chapter_count,
            entries.into_iter(),
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
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, title, url, content, cached_at, revision \
                 FROM chapter_cache",
            )
            .map_err(rusqlite_error)?;
        let entries = stmt
            .query_map([], row_to_chapter_cache_entry)
            .map_err(rusqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(rusqlite_error)?;
        Ok(chapter_cache_stats_from_entries(entries.iter()))
    }

    fn prune_chapter_cache(
        &self,
        policy: ChapterCacheRetentionPolicy,
    ) -> Result<ChapterCacheEvictionReport, StorageError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(rusqlite_error)?;

        let mut stmt = tx
            .prepare(
                "SELECT source_id, book_id, chapter_index, title, url, content, cached_at, revision \
                 FROM chapter_cache",
            )
            .map_err(rusqlite_error)?;
        let mut entries = stmt
            .query_map([], row_to_chapter_cache_entry)
            .map_err(rusqlite_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(rusqlite_error)?;
        drop(stmt);

        // Eviction order matches InMemoryStorage: oldest cached_at first, with
        // deterministic tie-breaking by (source_id, book_id, chapter_index).
        entries.sort_by(|a, b| {
            a.cached_at
                .cmp(&b.cached_at)
                .then_with(|| a.source_id.cmp(&b.source_id))
                .then_with(|| a.book_id.cmp(&b.book_id))
                .then_with(|| a.chapter_index.cmp(&b.chapter_index))
        });

        let mut evict_keys: Vec<(String, String, u32)> = Vec::new();
        let mut remaining: Vec<&ChapterCacheEntry> = Vec::with_capacity(entries.len());

        for entry in &entries {
            if let Some(min_cached_at) = policy.min_cached_at {
                if entry.cached_at < min_cached_at {
                    evict_keys.push((
                        entry.source_id.clone(),
                        entry.book_id.clone(),
                        entry.chapter_index,
                    ));
                    continue;
                }
            }
            remaining.push(entry);
        }

        if let Some(max_entries) = policy.max_entries {
            let overflow = remaining.len().saturating_sub(max_entries);
            if overflow > 0 {
                let evict_from_remaining: Vec<_> = remaining.drain(..overflow).collect();
                for entry in evict_from_remaining {
                    evict_keys.push((
                        entry.source_id.clone(),
                        entry.book_id.clone(),
                        entry.chapter_index,
                    ));
                }
            }
        }

        if let Some(max_total_content_bytes) = policy.max_total_content_bytes {
            let mut running = remaining
                .iter()
                .map(|e| crate::chapter_cache_content_bytes(e))
                .sum::<usize>();
            let mut additional_evict = Vec::new();
            // Walk remaining in order, evicting until the running total fits.
            // Uses an index loop instead of `take_while(|_| running > ...)` to
            // avoid borrowing `running` in the closure while we mutate it below.
            let mut idx = 0;
            while idx < remaining.len() && running > max_total_content_bytes {
                let entry = remaining[idx];
                running = running.saturating_sub(crate::chapter_cache_content_bytes(entry));
                additional_evict.push((
                    entry.source_id.clone(),
                    entry.book_id.clone(),
                    entry.chapter_index,
                ));
                idx += 1;
            }
            // Move evicted entries out of remaining.
            let evict_count = additional_evict.len();
            remaining.drain(..evict_count);
            evict_keys.extend(additional_evict);
        }

        let mut removed = Vec::with_capacity(evict_keys.len());
        for (source_id, book_id, chapter_index) in &evict_keys {
            let row = tx.query_row(
                "SELECT source_id, book_id, chapter_index, title, url, content, cached_at, revision \
                 FROM chapter_cache WHERE source_id = ?1 AND book_id = ?2 AND chapter_index = ?3",
                params![source_id, book_id, chapter_index],
                row_to_chapter_cache_entry,
            ).optional().map_err(rusqlite_error)?;
            if let Some(entry) = row {
                removed.push(entry);
                tx.execute(
                    "DELETE FROM chapter_cache \
                     WHERE source_id = ?1 AND book_id = ?2 AND chapter_index = ?3",
                    params![source_id, book_id, chapter_index],
                )
                .map_err(rusqlite_error)?;
            }
        }

        let remaining_stats = chapter_cache_stats_from_entries(remaining.iter().copied());
        tx.commit().map_err(rusqlite_error)?;
        Ok(ChapterCacheEvictionReport {
            removed,
            remaining: remaining_stats,
        })
    }
}

// ===== ReadingProgressStore =====

impl ReadingProgressStore for SqliteStorage {
    fn save_reading_progress(
        &self,
        entry: ReadingProgressEntry,
    ) -> Result<ReadingProgressEntry, StorageError> {
        validate_reading_progress(&entry)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(rusqlite_error)?;

        // Append to history with a per-book monotonic seq.
        let next_seq: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), -1) + 1 FROM reading_progress_history \
                 WHERE source_id = ?1 AND book_id = ?2",
                params![entry.source_id, entry.book_id],
                |row| row.get(0),
            )
            .map_err(rusqlite_error)?;
        tx.execute(
            "INSERT INTO reading_progress_history \
             (source_id, book_id, seq, chapter_index, chapter_offset, chapter_progress, updated_at, device_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.source_id,
                entry.book_id,
                next_seq,
                entry.chapter_index,
                entry.chapter_offset as i64,
                entry.chapter_progress,
                entry.updated_at,
                entry.device_id,
            ],
        )
        .map_err(rusqlite_error)?;

        // Advance current pointer only when updated_at is newer-or-equal.
        let should_advance: bool = tx
            .query_row(
                "SELECT updated_at FROM reading_progress WHERE source_id = ?1 AND book_id = ?2",
                params![entry.source_id, entry.book_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(rusqlite_error)?
            .flatten()
            .map(|current_updated| entry.updated_at >= current_updated)
            .unwrap_or(true);

        if should_advance {
            tx.execute(
                "INSERT OR REPLACE INTO reading_progress \
                 (source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry.source_id,
                    entry.book_id,
                    entry.chapter_index,
                    entry.chapter_offset as i64,
                    entry.chapter_progress,
                    entry.updated_at,
                    entry.device_id,
                ],
            )
            .map_err(rusqlite_error)?;
            // Mirror InMemoryStorage's cross-table side effect: update the
            // shelf entry's last_read_at when it exists.
            tx.execute(
                "UPDATE bookshelf SET last_read_at = ?1 \
                 WHERE source_id = ?2 AND book_id = ?3",
                params![entry.updated_at, entry.source_id, entry.book_id],
            )
            .map_err(rusqlite_error)?;
            tx.commit().map_err(rusqlite_error)?;
            Ok(entry)
        } else {
            // Read the current (newer) row back BEFORE commit, since
            // `Transaction::commit` takes ownership of `tx` and would
            // prevent a subsequent `tx.query_row(...)` call.
            let current = tx.query_row(
                "SELECT source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id \
                 FROM reading_progress WHERE source_id = ?1 AND book_id = ?2",
                params![entry.source_id, entry.book_id],
                row_to_reading_progress_entry,
            )
            .map_err(rusqlite_error)?;
            tx.commit().map_err(rusqlite_error)?;
            Ok(current)
        }
    }

    fn get_reading_progress(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Option<ReadingProgressEntry>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let conn = self.lock()?;
        conn.query_row(
            "SELECT source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id \
             FROM reading_progress WHERE source_id = ?1 AND book_id = ?2",
            params![source_id, book_id],
            row_to_reading_progress_entry,
        )
        .optional()
        .map_err(rusqlite_error)
    }

    fn list_reading_progress(
        &self,
        source_id: &str,
    ) -> Result<Vec<ReadingProgressEntry>, StorageError> {
        validate_source_id(source_id)?;
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id \
                 FROM reading_progress WHERE source_id = ?1 \
                 ORDER BY updated_at DESC, book_id ASC",
            )
            .map_err(rusqlite_error)?;
        let rows = stmt
            .query_map(params![source_id], row_to_reading_progress_entry)
            .map_err(rusqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(rusqlite_error)
    }

    fn reading_progress_history(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Vec<ReadingProgressEntry>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id \
                 FROM reading_progress_history WHERE source_id = ?1 AND book_id = ?2 \
                 ORDER BY updated_at ASC, device_id ASC, seq ASC",
            )
            .map_err(rusqlite_error)?;
        let rows = stmt
            .query_map(params![source_id, book_id], row_to_reading_progress_entry)
            .map_err(rusqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(rusqlite_error)
    }

    fn clear_reading_progress(&self, source_id: &str, book_id: &str) -> Result<(), StorageError> {
        validate_book_key(source_id, book_id)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(rusqlite_error)?;
        tx.execute(
            "DELETE FROM reading_progress WHERE source_id = ?1 AND book_id = ?2",
            params![source_id, book_id],
        )
        .map_err(rusqlite_error)?;
        tx.execute(
            "DELETE FROM reading_progress_history WHERE source_id = ?1 AND book_id = ?2",
            params![source_id, book_id],
        )
        .map_err(rusqlite_error)?;
        // Mirror InMemoryStorage: clear shelf.last_read_at if the shelf entry exists.
        tx.execute(
            "UPDATE bookshelf SET last_read_at = NULL WHERE source_id = ?1 AND book_id = ?2",
            params![source_id, book_id],
        )
        .map_err(rusqlite_error)?;
        tx.commit().map_err(rusqlite_error)?;
        Ok(())
    }
}

// ===== ChapterDownloadQueueStore =====

impl ChapterDownloadQueueStore for SqliteStorage {
    fn enqueue_chapter_download(
        &self,
        mut task: ChapterDownloadTask,
    ) -> Result<ChapterDownloadTask, StorageError> {
        validate_chapter_download_task(&task)?;
        let conn = self.lock()?;
        // Requeue semantics: preserve created_at, reset status/attempts/error.
        let created_at: i64 = conn
            .query_row(
                "SELECT created_at FROM chapter_download_queue \
                 WHERE source_id = ?1 AND book_id = ?2 AND chapter_index = ?3",
                params![task.source_id, task.book_id, task.chapter_index],
                |row| row.get(0),
            )
            .optional()
            .map_err(rusqlite_error)?
            .unwrap_or(task.created_at);
        task.created_at = created_at;
        task.status = ChapterDownloadStatus::Pending;
        task.attempts = 0;
        task.last_error = None;
        validate_chapter_download_task(&task)?;
        conn.execute(
            "INSERT OR REPLACE INTO chapter_download_queue \
             (source_id, book_id, chapter_index, title, url, priority, status, \
              created_at, updated_at, attempts, max_attempts, last_error) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                task.source_id,
                task.book_id,
                task.chapter_index,
                task.title,
                task.url,
                task.priority,
                download_status_to_db(&task.status),
                task.created_at,
                task.updated_at,
                task.attempts,
                task.max_attempts,
                task.last_error,
            ],
        )
        .map_err(rusqlite_error)?;
        Ok(task)
    }

    fn get_chapter_download(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
    ) -> Result<Option<ChapterDownloadTask>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let conn = self.lock()?;
        conn.query_row(
            "SELECT source_id, book_id, chapter_index, title, url, priority, status, \
             created_at, updated_at, attempts, max_attempts, last_error \
             FROM chapter_download_queue WHERE source_id = ?1 AND book_id = ?2 AND chapter_index = ?3",
            params![source_id, book_id, chapter_index],
            row_to_chapter_download_task,
        )
        .optional()
        .map_err(rusqlite_error)
    }

    fn list_chapter_downloads(
        &self,
        source_id: &str,
        book_id: &str,
    ) -> Result<Vec<ChapterDownloadTask>, StorageError> {
        validate_book_key(source_id, book_id)?;
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, title, url, priority, status, \
                 created_at, updated_at, attempts, max_attempts, last_error \
                 FROM chapter_download_queue WHERE source_id = ?1 AND book_id = ?2 \
                 ORDER BY chapter_index ASC",
            )
            .map_err(rusqlite_error)?;
        let rows = stmt
            .query_map(params![source_id, book_id], row_to_chapter_download_task)
            .map_err(rusqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(rusqlite_error)
    }

    fn claim_next_chapter_download(
        &self,
        now: i64,
    ) -> Result<Option<ChapterDownloadTask>, StorageError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(rusqlite_error)?;
        // Claimable = status IN (pending, failed) AND attempts < max_attempts.
        // Order matches InMemoryStorage: priority DESC, then updated_at ASC,
        // then created_at ASC, then (source_id, book_id, chapter_index).
        let next: Option<ChapterDownloadTask> = tx
            .query_row(
                "SELECT source_id, book_id, chapter_index, title, url, priority, status, \
                 created_at, updated_at, attempts, max_attempts, last_error \
                 FROM chapter_download_queue \
                 WHERE status IN ('pending', 'failed') AND attempts < max_attempts \
                 ORDER BY priority DESC, updated_at ASC, created_at ASC, \
                 source_id ASC, book_id ASC, chapter_index ASC \
                 LIMIT 1",
                [],
                row_to_chapter_download_task,
            )
            .optional()
            .map_err(rusqlite_error)?;

        let Some(mut task) = next else {
            return Ok(None);
        };
        task.status = ChapterDownloadStatus::InProgress;
        task.attempts += 1;
        task.updated_at = now;
        task.last_error = None;
        tx.execute(
            "UPDATE chapter_download_queue SET status = ?1, attempts = ?2, updated_at = ?3, last_error = NULL \
             WHERE source_id = ?4 AND book_id = ?5 AND chapter_index = ?6",
            params![
                download_status_to_db(&task.status),
                task.attempts,
                task.updated_at,
                task.source_id,
                task.book_id,
                task.chapter_index,
            ],
        )
        .map_err(rusqlite_error)?;
        tx.commit().map_err(rusqlite_error)?;
        Ok(Some(task))
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
        let conn = self.lock()?;
        let removed = conn
            .execute(
                "DELETE FROM chapter_download_queue \
                 WHERE source_id = ?1 AND book_id = ?2 \
                 AND status IN ('completed', 'cancelled')",
                params![source_id, book_id],
            )
            .map_err(rusqlite_error)?;
        Ok(removed)
    }
}

impl SqliteStorage {
    fn update_download_task(
        &self,
        source_id: &str,
        book_id: &str,
        chapter_index: u32,
        update: impl FnOnce(&mut ChapterDownloadTask),
    ) -> Result<ChapterDownloadTask, StorageError> {
        let conn = self.lock()?;
        let mut task = conn
            .query_row(
                "SELECT source_id, book_id, chapter_index, title, url, priority, status, \
                 created_at, updated_at, attempts, max_attempts, last_error \
                 FROM chapter_download_queue WHERE source_id = ?1 AND book_id = ?2 AND chapter_index = ?3",
                params![source_id, book_id, chapter_index],
                row_to_chapter_download_task,
            )
            .optional()
            .map_err(rusqlite_error)?
            .ok_or(StorageError::DownloadTaskNotFound {
                source_id: source_id.to_string(),
                book_id: book_id.to_string(),
                chapter_index,
            })?;
        update(&mut task);
        validate_chapter_download_task(&task)?;
        conn.execute(
            "UPDATE chapter_download_queue SET status = ?1, updated_at = ?2, attempts = ?3, \
             last_error = ?4 WHERE source_id = ?5 AND book_id = ?6 AND chapter_index = ?7",
            params![
                download_status_to_db(&task.status),
                task.updated_at,
                task.attempts,
                task.last_error,
                task.source_id,
                task.book_id,
                task.chapter_index,
            ],
        )
        .map_err(rusqlite_error)?;
        Ok(task)
    }
}

// ===== StorageSnapshotStore =====

impl StorageSnapshotStore for SqliteStorage {
    fn export_snapshot(&self, exported_at: i64) -> Result<StorageSnapshot, StorageError> {
        let conn = self.lock()?;
        let mut snapshot = StorageSnapshot {
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
        };

        let mut stmt = conn
            .prepare("SELECT source_json FROM sources")
            .map_err(rusqlite_error)?;
        let source_rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(rusqlite_error)?;
        for json in source_rows {
            let json = json.map_err(rusqlite_error)?;
            let source: Source =
                serde_json::from_str(&json).map_err(|_| StorageError::InvalidSnapshot {
                    field: "source_json".into(),
                })?;
            snapshot.sources.push(source);
        }
        drop(stmt);

        let mut stmt = conn
            .prepare("SELECT book_json FROM books")
            .map_err(rusqlite_error)?;
        let book_rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(rusqlite_error)?;
        for json in book_rows {
            let json = json.map_err(rusqlite_error)?;
            let book: Book =
                serde_json::from_str(&json).map_err(|_| StorageError::InvalidSnapshot {
                    field: "book_json".into(),
                })?;
            snapshot.books.push(book);
        }
        drop(stmt);

        let mut stmt = conn
            .prepare("SELECT key, payload FROM cache")
            .map_err(rusqlite_error)?;
        let cache_rows = stmt
            .query_map([], |row| {
                Ok(CachedEntry {
                    key: row.get(0)?,
                    payload: row.get(1)?,
                })
            })
            .map_err(rusqlite_error)?;
        for entry in cache_rows {
            snapshot.cache.push(entry.map_err(rusqlite_error)?);
        }
        drop(stmt);

        // Legacy progress: reading_progress rows under source_id = "legacy".
        let mut stmt = conn
            .prepare(
                "SELECT book_id, chapter_index, chapter_offset, chapter_progress \
                 FROM reading_progress WHERE source_id = 'legacy'",
            )
            .map_err(rusqlite_error)?;
        let legacy_rows = stmt.query_map([], |row| {
            Ok(ReadingProgress {
                book_id: row.get(0)?,
                chapter_index: row.get::<_, i64>(1)? as u32,
                chapter_offset: row.get::<_, i64>(2)? as u64,
                chapter_progress: row.get(3)?,
            })
        });
        for progress in legacy_rows.map_err(rusqlite_error)? {
            snapshot
                .legacy_progress
                .push(progress.map_err(rusqlite_error)?);
        }
        drop(stmt);

        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, title, author, cover_url, intro, kind, last_chapter, \
                 added_at, last_read_at, \"group\", sort_index FROM bookshelf",
            )
            .map_err(rusqlite_error)?;
        let shelf_rows = stmt.query_map([], row_to_bookshelf_entry);
        for entry in shelf_rows.map_err(rusqlite_error)? {
            snapshot.bookshelf.push(entry.map_err(rusqlite_error)?);
        }
        drop(stmt);

        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, title, url, content, cached_at, revision \
                 FROM chapter_cache",
            )
            .map_err(rusqlite_error)?;
        let cache_chapter_rows = stmt.query_map([], row_to_chapter_cache_entry);
        for entry in cache_chapter_rows.map_err(rusqlite_error)? {
            snapshot.chapter_cache.push(entry.map_err(rusqlite_error)?);
        }
        drop(stmt);

        // reading_progress excludes the legacy synthetic rows.
        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id \
                 FROM reading_progress WHERE source_id != 'legacy'",
            )
            .map_err(rusqlite_error)?;
        let progress_rows = stmt.query_map([], row_to_reading_progress_entry);
        for entry in progress_rows.map_err(rusqlite_error)? {
            snapshot
                .reading_progress
                .push(entry.map_err(rusqlite_error)?);
        }
        drop(stmt);

        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id \
                 FROM reading_progress_history WHERE source_id != 'legacy'",
            )
            .map_err(rusqlite_error)?;
        let history_rows = stmt.query_map([], row_to_reading_progress_entry);
        for entry in history_rows.map_err(rusqlite_error)? {
            snapshot
                .reading_progress_history
                .push(entry.map_err(rusqlite_error)?);
        }
        drop(stmt);

        let mut stmt = conn
            .prepare(
                "SELECT source_id, book_id, chapter_index, title, url, priority, status, \
                 created_at, updated_at, attempts, max_attempts, last_error \
                 FROM chapter_download_queue",
            )
            .map_err(rusqlite_error)?;
        let download_rows = stmt.query_map([], row_to_chapter_download_task);
        for task in download_rows.map_err(rusqlite_error)? {
            snapshot
                .chapter_download_queue
                .push(task.map_err(rusqlite_error)?);
        }
        drop(stmt);

        sort_storage_snapshot(&mut snapshot);
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn replace_with_snapshot(&self, snapshot: StorageSnapshot) -> Result<(), StorageError> {
        snapshot.validate()?;
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(rusqlite_error)?;
        tx.execute_batch(
            "DELETE FROM sources; DELETE FROM books; DELETE FROM cache; \
             DELETE FROM bookshelf; DELETE FROM chapter_cache; \
             DELETE FROM reading_progress; DELETE FROM reading_progress_history; \
             DELETE FROM chapter_download_queue;",
        )
        .map_err(rusqlite_error)?;

        for source in &snapshot.sources {
            let json =
                serde_json::to_string(source).map_err(|_| StorageError::InvalidSnapshot {
                    field: "source_json".into(),
                })?;
            tx.execute(
                "INSERT OR REPLACE INTO sources (source_id, source_json) VALUES (?1, ?2)",
                params![source.source_id, json],
            )
            .map_err(rusqlite_error)?;
        }
        for book in &snapshot.books {
            let json = serde_json::to_string(book).map_err(|_| StorageError::InvalidSnapshot {
                field: "book_json".into(),
            })?;
            tx.execute(
                "INSERT OR REPLACE INTO books (book_id, book_json) VALUES (?1, ?2)",
                params![book.book_id, json],
            )
            .map_err(rusqlite_error)?;
        }
        for entry in &snapshot.cache {
            tx.execute(
                "INSERT OR REPLACE INTO cache (key, payload) VALUES (?1, ?2)",
                params![entry.key, entry.payload],
            )
            .map_err(rusqlite_error)?;
        }
        for progress in &snapshot.legacy_progress {
            tx.execute(
                "INSERT OR REPLACE INTO reading_progress \
                 (source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id) \
                 VALUES ('legacy', ?1, ?2, ?3, ?4, 0, NULL)",
                params![
                    progress.book_id,
                    progress.chapter_index,
                    progress.chapter_offset as i64,
                    progress.chapter_progress,
                ],
            )
            .map_err(rusqlite_error)?;
        }
        for entry in &snapshot.bookshelf {
            tx.execute(
                "INSERT OR REPLACE INTO bookshelf \
                 (source_id, book_id, title, author, cover_url, intro, kind, last_chapter, \
                 added_at, last_read_at, \"group\", sort_index) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    entry.source_id,
                    entry.book_id,
                    entry.title,
                    entry.author,
                    entry.cover_url,
                    entry.intro,
                    entry.kind,
                    entry.last_chapter,
                    entry.added_at,
                    entry.last_read_at,
                    entry.group,
                    entry.sort_index,
                ],
            )
            .map_err(rusqlite_error)?;
        }
        for entry in &snapshot.chapter_cache {
            tx.execute(
                "INSERT OR REPLACE INTO chapter_cache \
                 (source_id, book_id, chapter_index, title, url, content, cached_at, revision) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    entry.source_id,
                    entry.book_id,
                    entry.chapter_index,
                    entry.title,
                    entry.url,
                    entry.content,
                    entry.cached_at,
                    entry.revision,
                ],
            )
            .map_err(rusqlite_error)?;
        }
        for entry in &snapshot.reading_progress {
            tx.execute(
                "INSERT OR REPLACE INTO reading_progress \
                 (source_id, book_id, chapter_index, chapter_offset, chapter_progress, updated_at, device_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    entry.source_id,
                    entry.book_id,
                    entry.chapter_index,
                    entry.chapter_offset as i64,
                    entry.chapter_progress,
                    entry.updated_at,
                    entry.device_id,
                ],
            )
            .map_err(rusqlite_error)?;
        }
        for (seq, entry) in snapshot.reading_progress_history.iter().enumerate() {
            tx.execute(
                "INSERT OR REPLACE INTO reading_progress_history \
                 (source_id, book_id, seq, chapter_index, chapter_offset, chapter_progress, updated_at, device_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    entry.source_id,
                    entry.book_id,
                    seq as i64,
                    entry.chapter_index,
                    entry.chapter_offset as i64,
                    entry.chapter_progress,
                    entry.updated_at,
                    entry.device_id,
                ],
            )
            .map_err(rusqlite_error)?;
        }
        for task in &snapshot.chapter_download_queue {
            tx.execute(
                "INSERT OR REPLACE INTO chapter_download_queue \
                 (source_id, book_id, chapter_index, title, url, priority, status, \
                 created_at, updated_at, attempts, max_attempts, last_error) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    task.source_id,
                    task.book_id,
                    task.chapter_index,
                    task.title,
                    task.url,
                    task.priority,
                    download_status_to_db(&task.status),
                    task.created_at,
                    task.updated_at,
                    task.attempts,
                    task.max_attempts,
                    task.last_error,
                ],
            )
            .map_err(rusqlite_error)?;
        }
        tx.commit().map_err(rusqlite_error)?;
        Ok(())
    }
}

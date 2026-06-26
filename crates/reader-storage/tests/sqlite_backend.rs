//! Integration tests for the `SqliteStorage` persistent backend.
//!
//! These tests only run when the `sqlite` feature is enabled. They mirror the
//! behavioral surface already covered by `InMemoryStorage`'s unit tests, plus
//! three concerns unique to a real on-disk backend:
//!
//! 1. **Cross-table side effects** — `save_reading_progress` must mirror the
//!    `InMemoryStorage` behavior of touching `bookshelf.last_read_at`.
//! 2. **claim ordering** — `claim_next_chapter_download` honors `priority DESC`
//!    then `updated_at ASC`.
//! 3. **Canonical snapshot hash stability** — feeding the same data into
//!    `InMemoryStorage` and `SqliteStorage` yields identical SHA-256 hashes.
//!    This is the S5 exit-criterion primitive: snapshot/import/export fixtures
//!    must be backend-agnostic.

#![cfg(feature = "sqlite")]

use reader_domain::{
    Book, Bookmark, DictRule, ReadingProgress, ReplaceRule, Source, SourceRules, TxtTocRule,
};
use reader_storage::{
    BookshelfEntry, BookshelfQuery, BookshelfSortBy, BookshelfSortDirection, BookshelfStore,
    ChapterCacheEntry, ChapterCacheRetentionPolicy, ChapterCacheStats, ChapterCacheStore,
    ChapterDownloadQueueStore, ChapterDownloadStatus, ChapterDownloadTask, InMemoryStorage,
    ReadingProgressEntry, ReadingProgressStore, SqliteStorage, StorageSnapshot,
    StorageSnapshotStore,
};
use serde_json::Value;

// ---------- helpers ----------

fn test_source(id: &str, name: &str) -> Source {
    Source {
        source_id: id.into(),
        name: name.into(),
        base_url: String::new(),
        rules: SourceRules::default(),
        book_source: Value::Null,
    }
}

fn test_book(id: &str, title: &str) -> Book {
    Book {
        book_id: id.into(),
        title: title.into(),
        author: "无名作者".into(),
        cover_url: None,
        intro: None,
        kind: None,
        last_chapter: None,
    }
}

fn test_shelf_entry(source: &str, book: &str, added_at: i64) -> BookshelfEntry {
    BookshelfEntry {
        source_id: source.into(),
        book_id: book.into(),
        title: format!("title-{book}"),
        author: "无名作者".into(),
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

fn test_cache_entry(source: &str, book: &str, idx: u32, cached_at: i64) -> ChapterCacheEntry {
    ChapterCacheEntry {
        source_id: source.into(),
        book_id: book.into(),
        chapter_index: idx,
        title: format!("chapter-{idx}"),
        url: format!("/ch/{idx}"),
        content: format!("正文{idx}"),
        cached_at,
        revision: None,
    }
}

fn test_progress_entry(
    source: &str,
    book: &str,
    idx: u32,
    updated_at: i64,
) -> ReadingProgressEntry {
    ReadingProgressEntry {
        source_id: source.into(),
        book_id: book.into(),
        chapter_index: idx,
        chapter_offset: 0,
        chapter_progress: 0.0,
        updated_at,
        device_id: None,
    }
}

fn test_download_task(source: &str, book: &str, idx: u32, priority: i32) -> ChapterDownloadTask {
    ChapterDownloadTask {
        source_id: source.into(),
        book_id: book.into(),
        chapter_index: idx,
        title: format!("chapter-{idx}"),
        url: format!("/ch/{idx}"),
        priority,
        status: ChapterDownloadStatus::Pending,
        created_at: 1_000,
        updated_at: 1_000,
        attempts: 0,
        max_attempts: 3,
        last_error: None,
    }
}

fn test_progress(book: &str, idx: u32) -> ReadingProgress {
    ReadingProgress {
        book_id: book.into(),
        chapter_index: idx,
        chapter_offset: 0,
        chapter_progress: 0.0,
    }
}

// ---------- backend primitives ----------

#[test]
fn sqlite_open_in_memory_initializes_schema_v2() {
    let storage = SqliteStorage::open_in_memory().expect("open in-memory");
    assert_eq!(storage.user_version().unwrap(), 2);
}

#[test]
fn sqlite_put_get_source_round_trips() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let src = test_source("src-a", "Source A");
    let stored = storage.put_source(src.clone()).unwrap();
    assert_eq!(stored, src);
    let fetched = storage.get_source("src-a").unwrap();
    assert_eq!(fetched.as_ref(), Some(&src));
    assert!(storage.get_source("missing").unwrap().is_none());
}

#[test]
fn sqlite_put_get_book_round_trips() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let book = test_book("b1", "Title 1");
    let stored = storage.put_book(book.clone()).unwrap();
    assert_eq!(stored, book);
    let fetched = storage.get_book("b1").unwrap();
    assert_eq!(fetched.as_ref(), Some(&book));
}

#[test]
fn sqlite_put_get_cache_round_trips() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let stored = storage.put_cache("k1", "payload-1").unwrap();
    assert_eq!(stored.key, "k1");
    assert_eq!(stored.payload, "payload-1");
    let fetched = storage.get_cache("k1").unwrap();
    assert_eq!(
        fetched.as_ref().map(|e| e.payload.as_str()),
        Some("payload-1")
    );
    // overwrite
    storage.put_cache("k1", "payload-2").unwrap();
    let fetched2 = storage.get_cache("k1").unwrap();
    assert_eq!(
        fetched2.as_ref().map(|e| e.payload.as_str()),
        Some("payload-2")
    );
}

// ---------- BookshelfStore ----------

#[test]
fn sqlite_shelf_round_trip_and_upsert() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let e1 = test_shelf_entry("src-a", "b1", 1_000);
    let stored = storage.add_to_shelf(e1.clone()).unwrap();
    assert_eq!(stored, e1);

    let fetched = storage.get_shelf_entry("src-a", "b1").unwrap().unwrap();
    assert_eq!(fetched, e1);
    assert_eq!(storage.shelf_count().unwrap(), 1);

    // upsert preserves added_at, overwrites other fields
    let mut e2 = e1.clone();
    e2.title = "updated-title".into();
    e2.author = "new-author".into();
    storage.add_to_shelf(e2.clone()).unwrap();
    let fetched2 = storage.get_shelf_entry("src-a", "b1").unwrap().unwrap();
    assert_eq!(fetched2.added_at, 1_000); // preserved
    assert_eq!(fetched2.title, "updated-title");

    // list_shelf
    let listed = storage.list_shelf().unwrap();
    assert_eq!(listed.len(), 1);

    // query_shelf by source
    let q = BookshelfQuery {
        source_id: Some("src-a".into()),
        ..BookshelfQuery::default()
    };
    let queried = storage.query_shelf(q).unwrap();
    assert_eq!(queried.len(), 1);

    // list_shelf_by_group
    storage
        .move_shelf_entry("src-a", "b1", Some("追更".into()), 5)
        .unwrap();
    let grouped = storage.list_shelf_by_group("追更").unwrap();
    assert_eq!(grouped.len(), 1);
    assert_eq!(grouped[0].group.as_deref(), Some("追更"));
    assert_eq!(grouped[0].sort_index, 5);

    // update_last_read
    storage.update_last_read("src-a", "b1", 2_500).unwrap();
    let after = storage.get_shelf_entry("src-a", "b1").unwrap().unwrap();
    assert_eq!(after.last_read_at, Some(2_500));

    // update_last_read on missing entry → NotFound
    let err = storage
        .update_last_read("src-a", "missing", 9_999)
        .unwrap_err();
    assert!(matches!(err, reader_storage::StorageError::NotFound { .. }));

    // remove_from_shelf is idempotent
    storage.remove_from_shelf("src-a", "b1").unwrap();
    storage.remove_from_shelf("src-a", "b1").unwrap();
    assert_eq!(storage.shelf_count().unwrap(), 0);
}

#[test]
fn sqlite_shelf_query_sorts_by_manual_then_added_at_desc() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage
        .add_to_shelf(test_shelf_entry("s", "a", 1_000))
        .unwrap();
    storage
        .add_to_shelf(test_shelf_entry("s", "b", 3_000))
        .unwrap();
    storage
        .add_to_shelf(test_shelf_entry("s", "c", 2_000))
        .unwrap();
    let listed = storage.list_shelf().unwrap();
    // Default sort: sort_index asc (all 0), then added_at desc → b, c, a
    let ids: Vec<&str> = listed.iter().map(|e| e.book_id.as_str()).collect();
    assert_eq!(ids, vec!["b", "c", "a"]);

    // explicit sort by AddedAt ascending
    let q = BookshelfQuery {
        sort_by: BookshelfSortBy::AddedAt,
        sort_direction: BookshelfSortDirection::Ascending,
        ..BookshelfQuery::default()
    };
    let queried = storage.query_shelf(q).unwrap();
    let ids2: Vec<&str> = queried.iter().map(|e| e.book_id.as_str()).collect();
    assert_eq!(ids2, vec!["a", "c", "b"]);
}

// ---------- ChapterCacheStore ----------

#[test]
fn sqlite_chapter_cache_round_trip() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let e1 = test_cache_entry("s", "b", 0, 1_000);
    let e2 = test_cache_entry("s", "b", 1, 1_100);
    let e3 = test_cache_entry("s", "b", 2, 1_200);
    storage.put_chapter_cache(e1.clone()).unwrap();
    storage.put_chapter_cache(e2.clone()).unwrap();
    storage.put_chapter_cache(e3.clone()).unwrap();

    let got = storage.get_chapter_cache("s", "b", 1).unwrap().unwrap();
    assert_eq!(got, e2);

    let listed = storage.list_chapter_cache("s", "b").unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].chapter_index, 0);
    assert_eq!(listed[2].chapter_index, 2);

    storage.remove_chapter_cache("s", "b", 1).unwrap();
    // idempotent remove
    storage.remove_chapter_cache("s", "b", 1).unwrap();
    assert!(storage.get_chapter_cache("s", "b", 1).unwrap().is_none());
    assert_eq!(storage.list_chapter_cache("s", "b").unwrap().len(), 2);

    let removed = storage.clear_chapter_cache("s", "b").unwrap();
    assert_eq!(removed, 2);
    assert_eq!(storage.list_chapter_cache("s", "b").unwrap().len(), 0);
}

#[test]
fn sqlite_chapter_cache_coverage_and_stats() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    for i in 0..5u32 {
        storage
            .put_chapter_cache(test_cache_entry("s", "b", i, 1_000 + i as i64))
            .unwrap();
    }
    let stats: ChapterCacheStats = storage.chapter_cache_stats().unwrap();
    assert_eq!(stats.entry_count, 5);
    assert_eq!(stats.oldest_cached_at, Some(1_000));
    assert_eq!(stats.newest_cached_at, Some(1_004));

    let coverage = storage.chapter_cache_coverage("s", "b", 10).unwrap();
    // 5 of 10 chapters cached
    assert_eq!(coverage.cached_count, 5);
    assert_eq!(coverage.chapter_count, 10);
    assert_eq!(coverage.missing_count, 5);
}

#[test]
fn sqlite_prune_chapter_cache_evicts_oldest_first_by_cached_at() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    // Insert with deliberately non-monotonic cached_at to verify sort order.
    storage
        .put_chapter_cache(test_cache_entry("s", "b", 0, 3_000))
        .unwrap();
    storage
        .put_chapter_cache(test_cache_entry("s", "b", 1, 1_000))
        .unwrap();
    storage
        .put_chapter_cache(test_cache_entry("s", "b", 2, 2_000))
        .unwrap();
    storage
        .put_chapter_cache(test_cache_entry("s", "b", 3, 5_000))
        .unwrap();
    storage
        .put_chapter_cache(test_cache_entry("s", "b", 4, 4_000))
        .unwrap();

    // Limit to 3 entries. Eviction order: oldest cached_at first.
    // Order: 1_000 (idx=1), 2_000 (idx=2), 3_000 (idx=0), 4_000 (idx=4), 5_000 (idx=3).
    // Evict the 2 oldest: idx=1 and idx=2. Remaining: idx=0, idx=3, idx=4.
    let report = storage
        .prune_chapter_cache(ChapterCacheRetentionPolicy {
            max_entries: Some(3),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(report.removed.len(), 2);
    let removed_idx: Vec<u32> = report.removed.iter().map(|e| e.chapter_index).collect();
    assert_eq!(removed_idx, vec![1, 2]);
    assert_eq!(report.remaining.entry_count, 3);

    let remaining = storage.list_chapter_cache("s", "b").unwrap();
    let remaining_idx: Vec<u32> = remaining.iter().map(|e| e.chapter_index).collect();
    assert_eq!(remaining_idx, vec![0, 3, 4]);
}

// ---------- ReadingProgressStore ----------

#[test]
fn sqlite_reading_progress_history_and_cross_table_shelf_side_effect() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage
        .add_to_shelf(test_shelf_entry("s", "b", 1_000))
        .unwrap();

    // First progress update at t=2_000
    let p1 = test_progress_entry("s", "b", 3, 2_000);
    let saved = storage.save_reading_progress(p1.clone()).unwrap();
    assert_eq!(saved, p1);

    // Cross-table side effect: shelf.last_read_at should be updated.
    let shelf = storage.get_shelf_entry("s", "b").unwrap().unwrap();
    assert_eq!(shelf.last_read_at, Some(2_000));

    // Second update at t=5_000 (newer)
    let p2 = test_progress_entry("s", "b", 4, 5_000);
    storage.save_reading_progress(p2.clone()).unwrap();
    let shelf2 = storage.get_shelf_entry("s", "b").unwrap().unwrap();
    assert_eq!(shelf2.last_read_at, Some(5_000));

    // history preserves insertion order (oldest first)
    let history = storage.reading_progress_history("s", "b").unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].updated_at, 2_000);
    assert_eq!(history[1].updated_at, 5_000);

    // current progress
    let current = storage.get_reading_progress("s", "b").unwrap().unwrap();
    assert_eq!(current.chapter_index, 4);
    assert_eq!(current.updated_at, 5_000);

    // list_reading_progress returns one entry
    let listed = storage.list_reading_progress("s").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].book_id, "b");
}

#[test]
fn sqlite_save_reading_progress_older_update_does_not_advance_current() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage
        .add_to_shelf(test_shelf_entry("s", "b", 1_000))
        .unwrap();

    let newer = test_progress_entry("s", "b", 5, 5_000);
    storage.save_reading_progress(newer.clone()).unwrap();
    // Save an older update — current pointer must NOT regress.
    let older = test_progress_entry("s", "b", 1, 1_000);
    storage.save_reading_progress(older).unwrap();
    let current = storage.get_reading_progress("s", "b").unwrap().unwrap();
    assert_eq!(current.chapter_index, 5);
    assert_eq!(current.updated_at, 5_000);
    // But history should still have both events.
    let history = storage.reading_progress_history("s", "b").unwrap();
    assert_eq!(history.len(), 2);
}

#[test]
fn sqlite_clear_reading_progress_clears_shelf_last_read_at() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage
        .add_to_shelf(test_shelf_entry("s", "b", 1_000))
        .unwrap();
    storage
        .save_reading_progress(test_progress_entry("s", "b", 3, 2_000))
        .unwrap();
    assert_eq!(
        storage
            .get_shelf_entry("s", "b")
            .unwrap()
            .unwrap()
            .last_read_at,
        Some(2_000)
    );
    // clear
    storage.clear_reading_progress("s", "b").unwrap();
    // idempotent
    storage.clear_reading_progress("s", "b").unwrap();
    assert!(storage.get_reading_progress("s", "b").unwrap().is_none());
    let history = storage.reading_progress_history("s", "b").unwrap();
    assert_eq!(history.len(), 0);
    // shelf entry itself remains, but last_read_at is reset.
    let shelf = storage.get_shelf_entry("s", "b").unwrap().unwrap();
    assert_eq!(shelf.last_read_at, None);
}

// ---------- ChapterDownloadQueueStore ----------

#[test]
fn sqlite_chapter_download_priority_claim_order() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    // Three tasks with different priorities. claim_next must return
    // highest priority first (priority DESC), then earliest updated_at.
    storage
        .enqueue_chapter_download(test_download_task("s", "b", 0, 1))
        .unwrap();
    storage
        .enqueue_chapter_download(test_download_task("s", "b", 1, 5))
        .unwrap();
    storage
        .enqueue_chapter_download(test_download_task("s", "b", 2, 3))
        .unwrap();

    // Claim #1 → highest priority (5), chapter 1.
    let claimed1 = storage.claim_next_chapter_download(2_000).unwrap().unwrap();
    assert_eq!(claimed1.chapter_index, 1);
    assert_eq!(claimed1.priority, 5);
    assert_eq!(claimed1.status, ChapterDownloadStatus::InProgress);

    // Claim #2 → next highest (3), chapter 2.
    let claimed2 = storage.claim_next_chapter_download(2_100).unwrap().unwrap();
    assert_eq!(claimed2.chapter_index, 2);
    assert_eq!(claimed2.priority, 3);

    // Claim #3 → lowest (1), chapter 0.
    let claimed3 = storage.claim_next_chapter_download(2_200).unwrap().unwrap();
    assert_eq!(claimed3.chapter_index, 0);
    assert_eq!(claimed3.priority, 1);

    // No more pending.
    let claimed4 = storage.claim_next_chapter_download(2_300).unwrap();
    assert!(claimed4.is_none());
}

#[test]
fn sqlite_chapter_download_lifecycle_complete_fail_cancel_clear() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage
        .enqueue_chapter_download(test_download_task("s", "b", 0, 1))
        .unwrap();
    storage
        .enqueue_chapter_download(test_download_task("s", "b", 1, 1))
        .unwrap();
    storage
        .enqueue_chapter_download(test_download_task("s", "b", 2, 1))
        .unwrap();

    // claim + complete chapter 0
    let claimed0 = storage.claim_next_chapter_download(1_000).unwrap().unwrap();
    assert_eq!(claimed0.chapter_index, 0);
    let completed = storage
        .mark_chapter_download_completed("s", "b", 0, 1_200)
        .unwrap();
    assert_eq!(completed.status, ChapterDownloadStatus::Completed);

    // claim + fail chapter 1
    let claimed1 = storage.claim_next_chapter_download(1_100).unwrap().unwrap();
    assert_eq!(claimed1.chapter_index, 1);
    let failed = storage
        .mark_chapter_download_failed("s", "b", 1, "network timeout", 1_300)
        .unwrap();
    assert_eq!(failed.status, ChapterDownloadStatus::Failed);
    assert_eq!(failed.last_error.as_deref(), Some("network timeout"));
    assert_eq!(failed.attempts, 1);

    // claim + cancel chapter 2
    let claimed2 = storage.claim_next_chapter_download(1_200).unwrap().unwrap();
    assert_eq!(claimed2.chapter_index, 2);
    let cancelled = storage.cancel_chapter_download("s", "b", 2, 1_400).unwrap();
    assert_eq!(cancelled.status, ChapterDownloadStatus::Cancelled);

    // clear_finished removes completed + cancelled (failed tasks remain in
    // the queue, matching InMemoryStorage semantics).
    let cleared = storage.clear_finished_chapter_downloads("s", "b").unwrap();
    assert_eq!(cleared, 2);
    let listed = storage.list_chapter_downloads("s", "b").unwrap();
    // The failed task is still there.
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, ChapterDownloadStatus::Failed);
}

// ---------- StorageSnapshotStore ----------

#[test]
fn sqlite_export_snapshot_round_trip_then_replace_clears_state() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.put_source(test_source("s1", "src-one")).unwrap();
    storage.put_book(test_book("b1", "Title 1")).unwrap();
    storage.put_cache("k1", "payload-1").unwrap();
    storage
        .add_to_shelf(test_shelf_entry("s1", "b1", 1_000))
        .unwrap();
    storage.put_progress(test_progress("b1", 3)).unwrap();
    storage
        .put_chapter_cache(test_cache_entry("s1", "b1", 3, 2_000))
        .unwrap();
    storage
        .save_reading_progress(test_progress_entry("s1", "b1", 3, 3_000))
        .unwrap();
    storage
        .enqueue_chapter_download(test_download_task("s1", "b1", 3, 1))
        .unwrap();

    let snap = storage.export_snapshot(5_000).unwrap();
    assert_eq!(
        snap.schema_version,
        reader_storage::STORAGE_SNAPSHOT_SCHEMA_VERSION
    );
    assert_eq!(snap.exported_at, 5_000);
    assert_eq!(snap.sources.len(), 1);
    assert_eq!(snap.bookshelf.len(), 1);
    assert_eq!(snap.chapter_cache.len(), 1);
    assert_eq!(snap.reading_progress.len(), 1);
    assert_eq!(snap.chapter_download_queue.len(), 1);

    // Now mutate state and then replace_with_snapshot should restore.
    storage
        .add_to_shelf(test_shelf_entry("s2", "b2", 9_999))
        .unwrap();
    assert_eq!(storage.shelf_count().unwrap(), 2);

    storage.replace_with_snapshot(snap.clone()).unwrap();
    // Replaced state should match the snapshot, not the post-export mutation.
    assert_eq!(storage.shelf_count().unwrap(), 1);
    let after = storage.get_shelf_entry("s1", "b1").unwrap().unwrap();
    assert_eq!(after.book_id, "b1");
    assert!(storage.get_shelf_entry("s2", "b2").unwrap().is_none());

    // canonical hash survives the round trip.
    let h1 = storage.canonical_snapshot_hash(5_000).unwrap();
    let snap2 = storage.export_snapshot(5_000).unwrap();
    assert_eq!(snap2, snap);
    let _ = h1; // smoke: hash call succeeds
}

#[test]
fn sqlite_replace_with_empty_snapshot_clears_all_state() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage
        .add_to_shelf(test_shelf_entry("s", "b", 1_000))
        .unwrap();
    storage
        .put_chapter_cache(test_cache_entry("s", "b", 0, 2_000))
        .unwrap();
    storage
        .save_reading_progress(test_progress_entry("s", "b", 0, 3_000))
        .unwrap();

    let empty = StorageSnapshot::empty(4_000);
    storage.replace_with_snapshot(empty).unwrap();

    assert_eq!(storage.shelf_count().unwrap(), 0);
    assert_eq!(storage.list_chapter_cache("s", "b").unwrap().len(), 0);
    assert!(storage.get_reading_progress("s", "b").unwrap().is_none());
}

// ---------- Canonical snapshot hash stability (S5 exit-criterion) ----------

fn hash_snapshot(snap: &StorageSnapshot) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(snap).expect("snapshot serialize");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn sqlite_and_inmemory_produce_identical_canonical_snapshot_hash() {
    // This is the S5 exit-criterion test: the same logical state fed into
    // either backend must yield the same canonical SHA-256. It proves the
    // snapshot/import/export fixture format is backend-agnostic.
    let exported_at = 1_700_000_000_i64;

    let mem = InMemoryStorage::new();
    let sql = SqliteStorage::open_in_memory().unwrap();

    // Seed identical state into both backends.
    let src = test_source("src-a", "Source A");
    mem.put_source(src.clone()).unwrap();
    sql.put_source(src).unwrap();

    let book = test_book("b1", "Book One");
    mem.put_book(book.clone()).unwrap();
    sql.put_book(book).unwrap();

    mem.put_cache("k1", "payload-1").unwrap();
    sql.put_cache("k1", "payload-1").unwrap();

    let shelf = test_shelf_entry("src-a", "b1", 1_000);
    mem.add_to_shelf(shelf.clone()).unwrap();
    sql.add_to_shelf(shelf).unwrap();

    let cache = test_cache_entry("src-a", "b1", 0, 1_500);
    mem.put_chapter_cache(cache.clone()).unwrap();
    sql.put_chapter_cache(cache).unwrap();

    let progress = test_progress_entry("src-a", "b1", 0, 2_000);
    mem.save_reading_progress(progress.clone()).unwrap();
    sql.save_reading_progress(progress).unwrap();

    // InMemoryStorage has no inherent canonical_snapshot_hash; compute it
    // from its exported snapshot (which is already sorted internally).
    let mem_snap = mem.export_snapshot(exported_at).unwrap();
    let mem_hash = hash_snapshot(&mem_snap);

    // SqliteStorage has the inherent helper that wraps export_snapshot +
    // SHA-256 in the same canonical way.
    let sql_hash = sql.canonical_snapshot_hash(exported_at).unwrap();

    assert_eq!(
        mem_hash, sql_hash,
        "InMemoryStorage and SqliteStorage must produce identical canonical snapshot hashes for the same state"
    );
}

#[test]
fn sqlite_canonical_snapshot_hash_is_deterministic_across_calls() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.put_source(test_source("s", "src")).unwrap();
    storage
        .add_to_shelf(test_shelf_entry("s", "b", 1_000))
        .unwrap();
    let h1 = storage.canonical_snapshot_hash(9_999).unwrap();
    let h2 = storage.canonical_snapshot_hash(9_999).unwrap();
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64); // SHA-256 hex length
}

#[test]
fn sqlite_canonical_snapshot_hash_changes_when_state_changes() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.put_source(test_source("s", "src")).unwrap();
    let before = storage.canonical_snapshot_hash(1).unwrap();
    storage.put_source(test_source("s2", "src2")).unwrap();
    let after = storage.canonical_snapshot_hash(1).unwrap();
    assert_ne!(before, after, "hash must change when state changes");
}

// ---------- v2 independent entities (S5 B6) ----------

fn sample_txt_toc_rule(id: i64, name: &str, serial: i32) -> TxtTocRule {
    TxtTocRule {
        id,
        name: name.into(),
        rule: r"##正文(.*)".into(),
        example: Some("示例".into()),
        serial_number: serial,
        enable: true,
    }
}

fn sample_bookmark(time: i64, book_name: &str) -> Bookmark {
    Bookmark {
        time,
        book_name: book_name.into(),
        book_author: "无名作者".into(),
        chapter_index: 3,
        chapter_pos: 128,
        chapter_name: "第三章".into(),
        book_text: "原文片段".into(),
        content: "用户批注".into(),
    }
}

fn sample_replace_rule(id: i64, name: &str, order: i32) -> ReplaceRule {
    ReplaceRule {
        id,
        name: name.into(),
        group: Some("净化".into()),
        pattern: r"广告.*".into(),
        replacement: String::new(),
        scope: Some("book1,book2".into()),
        scope_title: false,
        scope_content: true,
        exclude_scope: Some("exclude1".into()),
        is_enabled: true,
        is_regex: true,
        timeout_millisecond: 3000,
        order,
    }
}

fn sample_dict_rule(name: &str, sort: i32) -> DictRule {
    DictRule {
        name: name.into(),
        url_rule: "http://dict.example.com/?q={{key}}".into(),
        show_rule: "##释义".into(),
        enabled: true,
        sort_number: sort,
    }
}

#[test]
fn sqlite_v2_entity_snapshot_round_trips_through_replace_and_export() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Build a snapshot with all four v2 entity collections populated.
    // Insert in a deliberately non-sorted order to verify the export sorts.
    let mut snap = StorageSnapshot::empty(7_000);
    snap.txt_toc_rules.push(sample_txt_toc_rule(2, "toc-b", 10));
    snap.txt_toc_rules.push(sample_txt_toc_rule(1, "toc-a", 5));
    snap.txt_toc_rules.push(sample_txt_toc_rule(3, "toc-c", 5));
    snap.bookmarks.push(sample_bookmark(2_000, "书B"));
    snap.bookmarks.push(sample_bookmark(1_000, "书A"));
    snap.replace_rules.push(sample_replace_rule(200, "r2", 1));
    snap.replace_rules.push(sample_replace_rule(100, "r1", 1));
    snap.replace_rules.push(sample_replace_rule(300, "r3", 0));
    snap.dict_rules.push(sample_dict_rule("dict-b", 1));
    snap.dict_rules.push(sample_dict_rule("dict-a", 0));

    storage.replace_with_snapshot(snap.clone()).unwrap();

    let exported = storage.export_snapshot(7_000).unwrap();
    assert_eq!(exported.schema_version, 2);
    assert_eq!(exported.txt_toc_rules.len(), 3);
    assert_eq!(exported.bookmarks.len(), 2);
    assert_eq!(exported.replace_rules.len(), 3);
    assert_eq!(exported.dict_rules.len(), 2);

    // txt_toc_rules sorted by (serial_number, id): (5,1), (5,3), (10,2)
    let toc_ids: Vec<i64> = exported.txt_toc_rules.iter().map(|r| r.id).collect();
    assert_eq!(toc_ids, vec![1, 3, 2]);
    // bookmarks sorted by time
    let bm_times: Vec<i64> = exported.bookmarks.iter().map(|b| b.time).collect();
    assert_eq!(bm_times, vec![1_000, 2_000]);
    // replace_rules sorted by (order, id): (0,300), (1,100), (1,200)
    let rp_ids: Vec<i64> = exported.replace_rules.iter().map(|r| r.id).collect();
    assert_eq!(rp_ids, vec![300, 100, 200]);
    // dict_rules sorted by name
    let dn: Vec<&str> = exported
        .dict_rules
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(dn, vec!["dict-a", "dict-b"]);

    // Field-level fidelity: scope/group/exclude_scope round-trip (Option fields).
    let r1 = exported.replace_rules.iter().find(|r| r.id == 100).unwrap();
    assert_eq!(r1.group.as_deref(), Some("净化"));
    assert_eq!(r1.scope.as_deref(), Some("book1,book2"));
    assert_eq!(r1.exclude_scope.as_deref(), Some("exclude1"));
    assert!(r1.scope_content);
    assert!(!r1.scope_title);

    // TxtTocRule.example (Option) round-trips.
    let t1 = exported.txt_toc_rules.iter().find(|r| r.id == 1).unwrap();
    assert_eq!(t1.example.as_deref(), Some("示例"));

    // Bookmark full-field round-trip.
    let b1 = exported.bookmarks.iter().find(|b| b.time == 1_000).unwrap();
    assert_eq!(b1.book_name, "书A");
    assert_eq!(b1.chapter_pos, 128);
    assert_eq!(b1.content, "用户批注");
}

#[test]
fn sqlite_v2_entity_snapshot_rejects_duplicate_pk() {
    let mut snap = StorageSnapshot::empty(1);
    snap.txt_toc_rules.push(sample_txt_toc_rule(1, "a", 0));
    snap.txt_toc_rules.push(sample_txt_toc_rule(1, "b", 1)); // duplicate id
    let err = snap.validate().unwrap_err();
    assert!(matches!(
        err,
        reader_storage::StorageError::InvalidSnapshot { ref field } if field == "txt_toc_rules"
    ));

    let mut snap2 = StorageSnapshot::empty(1);
    snap2.dict_rules.push(sample_dict_rule("dup", 0));
    snap2.dict_rules.push(sample_dict_rule("dup", 1)); // duplicate name
    let err2 = snap2.validate().unwrap_err();
    assert!(matches!(
        err2,
        reader_storage::StorageError::InvalidSnapshot { ref field } if field == "dict_rules"
    ));
}

#[test]
fn sqlite_v2_entity_replace_clears_existing_rows() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let mut first = StorageSnapshot::empty(1);
    first
        .txt_toc_rules
        .push(sample_txt_toc_rule(100, "old-rule", 0));
    first.dict_rules.push(sample_dict_rule("old-dict", 0));
    storage.replace_with_snapshot(first).unwrap();
    assert_eq!(storage.export_snapshot(1).unwrap().txt_toc_rules.len(), 1);

    // Replace with a snapshot containing different entities; the old rows
    // must be cleared, not merged.
    let mut second = StorageSnapshot::empty(2);
    second
        .txt_toc_rules
        .push(sample_txt_toc_rule(200, "new-rule", 0));
    storage.replace_with_snapshot(second).unwrap();

    let exported = storage.export_snapshot(2).unwrap();
    assert_eq!(exported.txt_toc_rules.len(), 1);
    assert_eq!(exported.txt_toc_rules[0].id, 200);
    assert!(exported.dict_rules.is_empty());
}

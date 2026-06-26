//! CRUD tests for the four v2 independent entities (TxtTocRule / Bookmark /
//! ReplaceRule / DictRule) on both `InMemoryStorage` and `SqliteStorage`.
//!
//! These verify that the two backends expose the same CRUD surface and
//! produce the same ordered results, so snapshot round-trips and direct
//! per-entity operations stay backend-agnostic. Mirrors Legado's per-entity
//! DAO semantics (ReplaceRuleDao / DictRuleDao / TxtTocRuleDao / BookmarkDao).

#![cfg(feature = "sqlite")]

use reader_domain::{Bookmark, DictRule, ReplaceRule, TxtTocRule};
use reader_storage::{InMemoryStorage, SqliteStorage, StorageSnapshotStore};

// ---------- sample factories ----------

fn txt(id: i64, name: &str, serial: i32, enable: bool) -> TxtTocRule {
    TxtTocRule {
        id,
        name: name.into(),
        rule: format!(r"##正文{}", id),
        example: Some(format!("ex{}", id)),
        serial_number: serial,
        enable,
    }
}

fn bm(time: i64, book_name: &str, chapter_name: &str, content: &str) -> Bookmark {
    Bookmark {
        time,
        book_name: book_name.into(),
        book_author: "无名作者".into(),
        chapter_index: (time % 10) as i32,
        chapter_pos: 0,
        chapter_name: chapter_name.into(),
        book_text: format!("text-{}", time),
        content: content.into(),
    }
}

fn rep(id: i64, name: &str, order: i32, enabled: bool, scope: Option<&str>) -> ReplaceRule {
    ReplaceRule {
        id,
        name: name.into(),
        group: Some("净化".into()),
        pattern: format!("pat{}", id),
        replacement: "".into(),
        scope: scope.map(str::to_string),
        scope_title: false,
        scope_content: true,
        exclude_scope: None,
        is_enabled: enabled,
        is_regex: true,
        timeout_millisecond: 3000,
        order,
    }
}

fn dict(name: &str, sort: i32) -> DictRule {
    DictRule {
        name: name.into(),
        url_rule: format!("http://d/?q={{}}&n={}", sort),
        show_rule: "##释义".into(),
        enabled: true,
        sort_number: sort,
    }
}

// ---------- TxtTocRule ----------

#[test]
fn inmemory_txt_toc_rule_crud_round_trip_and_enable_filter() {
    let store = InMemoryStorage::new();
    store.put_txt_toc_rule(txt(2, "b", 10, true)).unwrap();
    store.put_txt_toc_rule(txt(1, "a", 5, true)).unwrap();
    store.put_txt_toc_rule(txt(3, "c", 5, false)).unwrap();

    let got = store.get_txt_toc_rule(1).unwrap().unwrap();
    assert_eq!(got.name, "a");
    assert!(store.get_txt_toc_rule(999).unwrap().is_none());

    // list sorts by (serial_number, id): (5,1), (5,3), (10,2)
    let all = store.list_txt_toc_rules().unwrap();
    let ids: Vec<i64> = all.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![1, 3, 2]);

    // enabled filter excludes id=3
    let enabled = store.list_enabled_txt_toc_rules().unwrap();
    let eids: Vec<i64> = enabled.iter().map(|r| r.id).collect();
    assert_eq!(eids, vec![1, 2]);

    store.delete_txt_toc_rule(1).unwrap();
    assert_eq!(store.list_txt_toc_rules().unwrap().len(), 2);
    // idempotent delete
    store.delete_txt_toc_rule(1).unwrap();
}

#[test]
fn sqlite_txt_toc_rule_crud_round_trip_and_enable_filter() {
    let store = SqliteStorage::open_in_memory().unwrap();
    store.put_txt_toc_rule(txt(2, "b", 10, true)).unwrap();
    store.put_txt_toc_rule(txt(1, "a", 5, true)).unwrap();
    store.put_txt_toc_rule(txt(3, "c", 5, false)).unwrap();

    let got = store.get_txt_toc_rule(1).unwrap().unwrap();
    assert_eq!(got.example.as_deref(), Some("ex1"));
    assert!(store.get_txt_toc_rule(999).unwrap().is_none());

    let all = store.list_txt_toc_rules().unwrap();
    let ids: Vec<i64> = all.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![1, 3, 2]);

    let enabled = store.list_enabled_txt_toc_rules().unwrap();
    let eids: Vec<i64> = enabled.iter().map(|r| r.id).collect();
    assert_eq!(eids, vec![1, 2]);

    store.delete_txt_toc_rule(1).unwrap();
    assert_eq!(store.list_txt_toc_rules().unwrap().len(), 2);
}

// ---------- Bookmark ----------

#[test]
fn inmemory_bookmark_crud_by_book_and_search() {
    let store = InMemoryStorage::new();
    store
        .put_bookmark(bm(2_000, "书A", "第一章", "笔记A1"))
        .unwrap();
    store
        .put_bookmark(bm(1_000, "书A", "第二章", "笔记A2"))
        .unwrap();
    store
        .put_bookmark(bm(3_000, "书B", "第一章", "特殊标记"))
        .unwrap();

    // by-book filter + time sort
    let a_bms = store.list_bookmarks_by_book("书A", "无名作者").unwrap();
    let times: Vec<i64> = a_bms.iter().map(|b| b.time).collect();
    assert_eq!(times, vec![1_000, 2_000]);

    // search across book_name/chapter_name/content/book_text
    let hits = store.search_bookmarks("特殊").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].time, 3_000);
    let hits2 = store.search_bookmarks("书A").unwrap();
    assert_eq!(hits2.len(), 2);

    store.delete_bookmark(1_000).unwrap();
    assert_eq!(store.list_bookmarks().unwrap().len(), 2);
}

#[test]
fn sqlite_bookmark_crud_by_book_and_search() {
    let store = SqliteStorage::open_in_memory().unwrap();
    store
        .put_bookmark(bm(2_000, "书A", "第一章", "笔记A1"))
        .unwrap();
    store
        .put_bookmark(bm(1_000, "书A", "第二章", "笔记A2"))
        .unwrap();
    store
        .put_bookmark(bm(3_000, "书B", "第一章", "特殊标记"))
        .unwrap();

    let a_bms = store.list_bookmarks_by_book("书A", "无名作者").unwrap();
    let times: Vec<i64> = a_bms.iter().map(|b| b.time).collect();
    assert_eq!(times, vec![1_000, 2_000]);

    let hits = store.search_bookmarks("特殊").unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].time, 3_000);

    store.delete_bookmark(1_000).unwrap();
    assert_eq!(store.list_bookmarks().unwrap().len(), 2);
}

// ---------- ReplaceRule ----------

#[test]
fn inmemory_replace_rule_crud_and_ordering() {
    let store = InMemoryStorage::new();
    store
        .put_replace_rule(rep(200, "r2", 1, true, Some("book1")))
        .unwrap();
    store
        .put_replace_rule(rep(100, "r1", 1, true, None))
        .unwrap();
    store
        .put_replace_rule(rep(300, "r3", 0, false, None))
        .unwrap();

    let all = store.list_replace_rules().unwrap();
    // sort by (order, id): (0,300), (1,100), (1,200)
    let ids: Vec<i64> = all.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![300, 100, 200]);

    // scope field round-trips
    let r2 = store.get_replace_rule(200).unwrap().unwrap();
    assert_eq!(r2.scope.as_deref(), Some("book1"));
    assert_eq!(r2.group.as_deref(), Some("净化"));

    store.delete_replace_rule(100).unwrap();
    assert_eq!(store.list_replace_rules().unwrap().len(), 2);
}

#[test]
fn sqlite_replace_rule_crud_and_ordering() {
    let store = SqliteStorage::open_in_memory().unwrap();
    store
        .put_replace_rule(rep(200, "r2", 1, true, Some("book1")))
        .unwrap();
    store
        .put_replace_rule(rep(100, "r1", 1, true, None))
        .unwrap();
    store
        .put_replace_rule(rep(300, "r3", 0, false, None))
        .unwrap();

    let all = store.list_replace_rules().unwrap();
    let ids: Vec<i64> = all.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![300, 100, 200]);

    let r2 = store.get_replace_rule(200).unwrap().unwrap();
    assert_eq!(r2.scope.as_deref(), Some("book1"));
    assert!(!r2.scope_title);
    assert!(r2.scope_content);

    store.delete_replace_rule(100).unwrap();
    assert_eq!(store.list_replace_rules().unwrap().len(), 2);
}

// ---------- DictRule ----------

#[test]
fn inmemory_dict_rule_crud_and_name_validation() {
    let store = InMemoryStorage::new();
    store.put_dict_rule(dict("dict-b", 1)).unwrap();
    store.put_dict_rule(dict("dict-a", 0)).unwrap();

    let all = store.list_dict_rules().unwrap();
    let names: Vec<&str> = all.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["dict-a", "dict-b"]);

    let got = store.get_dict_rule("dict-a").unwrap().unwrap();
    assert_eq!(got.sort_number, 0);

    // empty name rejected
    let err = store.put_dict_rule(dict("", 0)).unwrap_err();
    assert!(matches!(
        err,
        reader_storage::StorageError::InvalidKey { ref field } if field == "dict_rules.name"
    ));

    store.delete_dict_rule("dict-a").unwrap();
    assert_eq!(store.list_dict_rules().unwrap().len(), 1);
}

#[test]
fn sqlite_dict_rule_crud_and_name_validation() {
    let store = SqliteStorage::open_in_memory().unwrap();
    store.put_dict_rule(dict("dict-b", 1)).unwrap();
    store.put_dict_rule(dict("dict-a", 0)).unwrap();

    let all = store.list_dict_rules().unwrap();
    let names: Vec<&str> = all.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["dict-a", "dict-b"]);

    let got = store.get_dict_rule("dict-a").unwrap().unwrap();
    assert_eq!(got.url_rule, "http://d/?q={}&n=0");

    let err = store.put_dict_rule(dict("", 0)).unwrap_err();
    assert!(matches!(
        err,
        reader_storage::StorageError::InvalidKey { ref field } if field == "dict_rules.name"
    ));

    store.delete_dict_rule("dict-a").unwrap();
    assert_eq!(store.list_dict_rules().unwrap().len(), 1);
}

// ---------- backend parity: CRUD state reflected in snapshot ----------

#[test]
fn inmemory_and_sqlite_crud_produce_identical_snapshot_for_entities() {
    let mem = InMemoryStorage::new();
    let sql = SqliteStorage::open_in_memory().unwrap();

    // Seed identical entity state via CRUD into both backends.
    for rule in [
        txt(1, "a", 5, true),
        txt(2, "b", 10, true),
        txt(3, "c", 5, false),
    ] {
        mem.put_txt_toc_rule(rule.clone()).unwrap();
        sql.put_txt_toc_rule(rule).unwrap();
    }
    for bookmark in [
        bm(1_000, "书A", "第一章", "n1"),
        bm(2_000, "书B", "第二章", "n2"),
    ] {
        mem.put_bookmark(bookmark.clone()).unwrap();
        sql.put_bookmark(bookmark).unwrap();
    }
    for rule in [
        rep(100, "r1", 0, true, None),
        rep(200, "r2", 1, true, Some("s")),
    ] {
        mem.put_replace_rule(rule.clone()).unwrap();
        sql.put_replace_rule(rule).unwrap();
    }
    for rule in [dict("d1", 0), dict("d2", 1)] {
        mem.put_dict_rule(rule.clone()).unwrap();
        sql.put_dict_rule(rule).unwrap();
    }

    let mem_snap = mem.export_snapshot(9_000).unwrap();
    let sql_snap = sql.export_snapshot(9_000).unwrap();

    // The four v2 entity collections must match exactly between backends
    // (ordering is canonicalized by sort_storage_snapshot in both paths).
    assert_eq!(mem_snap.txt_toc_rules, sql_snap.txt_toc_rules);
    assert_eq!(mem_snap.bookmarks, sql_snap.bookmarks);
    assert_eq!(mem_snap.replace_rules, sql_snap.replace_rules);
    assert_eq!(mem_snap.dict_rules, sql_snap.dict_rules);
}

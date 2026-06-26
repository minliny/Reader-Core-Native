//! Tests for the four independent configurable entities (TxtTocRule, Bookmark,
//! ReplaceRule, DictRule) and ReplaceRule scope filtering.
//!
//! Baseline alignment:
//! - Legado `app/src/main/java/io/legado/app/data/entities/{TxtTocRule,Bookmark,ReplaceRule,DictRule}.kt`
//! - Swift `ReaderCore/Sources/ReaderCoreModels/DictRule.swift`,
//!   `ReaderCore/Sources/ReaderCoreParser/ReplaceRuleEngine.swift`
//!   (ReaderCoreManagedReplaceRule + scope filtering).

use reader_domain::{
    scope_tokens, Bookmark, DictRule, ReplaceRule, ReplaceRuleEvaluationContext, ReplaceRuleTarget,
    TxtTocRule,
};
use serde_json::json;

// ---------------------------------------------------------------------------
// TxtTocRule (against Legado TxtTocRule.kt)
// ---------------------------------------------------------------------------

#[test]
fn txt_toc_rule_serde_roundtrip_full() {
    let rule = TxtTocRule {
        id: 1_700_000_000_000,
        name: "中文章节".to_string(),
        rule: r"^第[一二三四五六七八九十百千零〇0-9]+章".to_string(),
        example: Some("第一章 风起".to_string()),
        serial_number: 10,
        enable: true,
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(
        json,
        json!({
            "id": 1_700_000_000_000_i64,
            "name": "中文章节",
            "rule": r"^第[一二三四五六七八九十百千零〇0-9]+章",
            "example": "第一章 风起",
            "serialNumber": 10,
            "enable": true
        })
    );
    let back: TxtTocRule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn txt_toc_rule_defaults_match_legado() {
    // Legado: serialNumber = -1, enable = true
    let json = json!({ "id": 42, "name": "n", "rule": "r" });
    let rule: TxtTocRule = serde_json::from_value(json).unwrap();
    assert_eq!(rule.id, 42);
    assert_eq!(rule.example, None);
    assert_eq!(rule.serial_number, -1);
    assert!(rule.enable);
}

#[test]
fn txt_toc_rule_deny_unknown_fields() {
    let json = json!({ "id": 1, "name": "n", "rule": "r", "bogus": 7 });
    let err = serde_json::from_value::<TxtTocRule>(json).unwrap_err();
    assert!(err.to_string().contains("unknown field"), "{}", err);
}

// ---------------------------------------------------------------------------
// Bookmark (against Legado Bookmark.kt)
// ---------------------------------------------------------------------------

#[test]
fn bookmark_serde_roundtrip_full() {
    let bm = Bookmark {
        time: 1_700_000_000_001,
        book_name: "捞尸人".to_string(),
        book_author: "作者".to_string(),
        chapter_index: 5,
        chapter_pos: 1234,
        chapter_name: "第五章 入水".to_string(),
        book_text: "正文片段…".to_string(),
        content: "用户批注".to_string(),
    };
    let json = serde_json::to_value(&bm).unwrap();
    assert_eq!(
        json,
        json!({
            "time": 1_700_000_000_001_i64,
            "bookName": "捞尸人",
            "bookAuthor": "作者",
            "chapterIndex": 5,
            "chapterPos": 1234,
            "chapterName": "第五章 入水",
            "bookText": "正文片段…",
            "content": "用户批注"
        })
    );
    let back: Bookmark = serde_json::from_value(json).unwrap();
    assert_eq!(back, bm);
}

#[test]
fn bookmark_defaults_match_legado() {
    let json = json!({ "time": 99 });
    let bm: Bookmark = serde_json::from_value(json).unwrap();
    assert_eq!(bm.time, 99);
    assert_eq!(bm.book_name, "");
    assert_eq!(bm.book_author, "");
    assert_eq!(bm.chapter_index, 0);
    assert_eq!(bm.chapter_pos, 0);
    assert_eq!(bm.chapter_name, "");
    assert_eq!(bm.book_text, "");
    assert_eq!(bm.content, "");
}

#[test]
fn bookmark_deny_unknown_fields() {
    let json = json!({ "time": 1, "nope": true });
    let err = serde_json::from_value::<Bookmark>(json).unwrap_err();
    assert!(err.to_string().contains("unknown field"), "{}", err);
}

// ---------------------------------------------------------------------------
// ReplaceRule (against Swift ReaderCoreManagedReplaceRule + Legado ReplaceRule.kt)
// ---------------------------------------------------------------------------

#[test]
fn replace_rule_serde_roundtrip_full() {
    let rule = ReplaceRule {
        id: 1_700_000_000_002,
        name: "净化广告".to_string(),
        group: Some("默认".to_string()),
        pattern: r"<ad>.*?</ad>".to_string(),
        replacement: "".to_string(),
        scope: Some("bookA,sourceB".to_string()),
        scope_title: false,
        scope_content: true,
        exclude_scope: Some("bookC".to_string()),
        is_enabled: true,
        is_regex: true,
        timeout_millisecond: 3000,
        order: 5,
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(
        json,
        json!({
            "id": 1_700_000_000_002_i64,
            "name": "净化广告",
            "group": "默认",
            "pattern": r"<ad>.*?</ad>",
            "replacement": "",
            "scope": "bookA,sourceB",
            "scopeTitle": false,
            "scopeContent": true,
            "excludeScope": "bookC",
            "isEnabled": true,
            "isRegex": true,
            "timeoutMillisecond": 3000,
            "order": 5
        })
    );
    let back: ReplaceRule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn replace_rule_defaults_match_legado_swift() {
    // Legado: isEnabled=true, isRegex=true, timeout=3000, scopeContent=true,
    // scopeTitle=false, order=0; Swift: scopeTitle=false, scopeContent=true,
    // isEnabled=true, timeout=1000 — we follow Legado (3000) since charter red
    // line 3 says Legado is the baseline.
    let json = json!({ "id": 7, "pattern": "p" });
    let rule: ReplaceRule = serde_json::from_value(json).unwrap();
    assert_eq!(rule.id, 7);
    assert_eq!(rule.name, "");
    assert_eq!(rule.group, None);
    assert_eq!(rule.replacement, "");
    assert_eq!(rule.scope, None);
    assert!(!rule.scope_title);
    assert!(rule.scope_content);
    assert_eq!(rule.exclude_scope, None);
    assert!(rule.is_enabled);
    assert!(rule.is_regex);
    assert_eq!(rule.timeout_millisecond, 3000);
    assert_eq!(rule.order, 0);
}

#[test]
fn replace_rule_deny_unknown_fields() {
    let json = json!({ "id": 1, "pattern": "p", "extra": 1 });
    let err = serde_json::from_value::<ReplaceRule>(json).unwrap_err();
    assert!(err.to_string().contains("unknown field"), "{}", err);
}

// ---------------------------------------------------------------------------
// ReplaceRule scope filtering (against Swift matches_target / matches_include_scope /
// matches_exclude_scope). ANY semantics; token split by , ; |; substring match.
// ---------------------------------------------------------------------------

#[test]
fn scope_tokens_split_by_semicolon_comma_pipe_and_lowercase() {
    let tokens = scope_tokens(Some("BookA, sourceB;bookC|SourceD"));
    assert_eq!(
        tokens,
        vec![
            "booka".to_string(),
            "sourceb".to_string(),
            "bookc".to_string(),
            "sourced".to_string()
        ]
    );
}

#[test]
fn scope_tokens_empty_when_none_or_blank() {
    assert!(scope_tokens(None).is_empty());
    assert!(scope_tokens(Some("")).is_empty());
    assert!(scope_tokens(Some("   ,, ; ")).is_empty());
}

#[test]
fn replace_rule_matches_target_title_and_content() {
    let mut rule = ReplaceRule {
        id: 1,
        name: "n".into(),
        group: None,
        pattern: "p".into(),
        replacement: "".into(),
        scope: None,
        scope_title: false,
        scope_content: true,
        exclude_scope: None,
        is_enabled: true,
        is_regex: true,
        timeout_millisecond: 3000,
        order: 0,
    };
    // scope_content=true, scope_title=false → matches content, not title
    assert!(reader_domain::replace_rule_matches_target(
        &rule,
        ReplaceRuleTarget::Content
    ));
    assert!(!reader_domain::replace_rule_matches_target(
        &rule,
        ReplaceRuleTarget::Title
    ));

    rule.scope_title = true;
    rule.scope_content = false;
    assert!(reader_domain::replace_rule_matches_target(
        &rule,
        ReplaceRuleTarget::Title
    ));
    assert!(!reader_domain::replace_rule_matches_target(
        &rule,
        ReplaceRuleTarget::Content
    ));
}

#[test]
fn replace_rule_empty_scope_matches_all() {
    let rule = ReplaceRule {
        id: 1,
        name: "n".into(),
        group: None,
        pattern: "p".into(),
        replacement: "".into(),
        scope: None,
        scope_title: false,
        scope_content: true,
        exclude_scope: None,
        is_enabled: true,
        is_regex: true,
        timeout_millisecond: 3000,
        order: 0,
    };
    let ctx = ReplaceRuleEvaluationContext {
        book_title: "AnyBook".into(),
        source_name: "AnySource".into(),
        source_url: "https://example.test/s".into(),
    };
    assert!(reader_domain::replace_rule_matches_scope(&rule, &ctx));
}

#[test]
fn replace_rule_include_scope_any_semantics_substring() {
    let rule = ReplaceRule {
        id: 1,
        name: "n".into(),
        group: None,
        pattern: "p".into(),
        replacement: "".into(),
        scope: Some("booka,sourceb".into()),
        scope_title: false,
        scope_content: true,
        exclude_scope: None,
        is_enabled: true,
        is_regex: true,
        timeout_millisecond: 3000,
        order: 0,
    };
    // book title contains "booka" → match
    let ctx_hit = ReplaceRuleEvaluationContext {
        book_title: "MyBookA Special".into(),
        source_name: "x".into(),
        source_url: "y".into(),
    };
    assert!(reader_domain::replace_rule_matches_scope(&rule, &ctx_hit));
    // source name contains "sourceb" → match
    let ctx_hit2 = ReplaceRuleEvaluationContext {
        book_title: "nope".into(),
        source_name: "SourceB Alpha".into(),
        source_url: "y".into(),
    };
    assert!(reader_domain::replace_rule_matches_scope(&rule, &ctx_hit2));
    // neither → no match
    let ctx_miss = ReplaceRuleEvaluationContext {
        book_title: "nope".into(),
        source_name: "other".into(),
        source_url: "y".into(),
    };
    assert!(!reader_domain::replace_rule_matches_scope(&rule, &ctx_miss));
}

#[test]
fn replace_rule_exclude_scope_overrides_include() {
    let rule = ReplaceRule {
        id: 1,
        name: "n".into(),
        group: None,
        pattern: "p".into(),
        replacement: "".into(),
        scope: Some("booka".into()),
        scope_title: false,
        scope_content: true,
        exclude_scope: Some("forbidden".into()),
        is_enabled: true,
        is_regex: true,
        timeout_millisecond: 3000,
        order: 0,
    };
    // matches include but also matches exclude → excluded
    let ctx = ReplaceRuleEvaluationContext {
        book_title: "BookA Forbidden".into(),
        source_name: "s".into(),
        source_url: "u".into(),
    };
    assert!(!reader_domain::replace_rule_matches_scope(&rule, &ctx));
}

// ---------------------------------------------------------------------------
// DictRule (against Swift DictRule.swift + Legado DictRule.kt)
// ---------------------------------------------------------------------------

#[test]
fn dict_rule_serde_roundtrip_full() {
    let rule = DictRule {
        name: "dictA".to_string(),
        url_rule: "https://dict.example.test/q?key={{key}}".to_string(),
        show_rule: "//div.meaning".to_string(),
        enabled: true,
        sort_number: 3,
    };
    let json = serde_json::to_value(&rule).unwrap();
    assert_eq!(
        json,
        json!({
            "name": "dictA",
            "urlRule": "https://dict.example.test/q?key={{key}}",
            "showRule": "//div.meaning",
            "enabled": true,
            "sortNumber": 3
        })
    );
    let back: DictRule = serde_json::from_value(json).unwrap();
    assert_eq!(back, rule);
}

#[test]
fn dict_rule_defaults_match_legado() {
    // Legado: enabled=true, sortNumber=0
    let json = json!({ "name": "n" });
    let rule: DictRule = serde_json::from_value(json).unwrap();
    assert_eq!(rule.name, "n");
    assert_eq!(rule.url_rule, "");
    assert_eq!(rule.show_rule, "");
    assert!(rule.enabled);
    assert_eq!(rule.sort_number, 0);
}

#[test]
fn dict_rule_deny_unknown_fields() {
    let json = json!({ "name": "n", "bogus": 1 });
    let err = serde_json::from_value::<DictRule>(json).unwrap_err();
    assert!(err.to_string().contains("unknown field"), "{}", err);
}

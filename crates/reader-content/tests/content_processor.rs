//! ContentProcessor tests — Legado `ContentProcessor.kt:91` getContent() 替换规则
//! 管线的 Rust 落地。对照 Legado `data/entities/ReplaceRule.kt` 字段语义 +
//! `ReplaceRuleDao` 的 scope/excludeScope 过滤 + `getContent()` 的逐条替换流程。
//!
//! 证据级别:crate test(章程 §9 第 5 问)。

use reader_content::ContentProcessor;
use reader_domain::ReplaceRule;

/// 构造一条替换规则,字段对齐 Legado `ReplaceRule.kt`。
fn rule(id: i64, pattern: &str, replacement: &str) -> ReplaceRule {
    ReplaceRule {
        id,
        name: format!("rule-{id}"),
        group: None,
        pattern: pattern.to_string(),
        replacement: replacement.to_string(),
        scope: None,
        scope_title: false,
        scope_content: true,
        exclude_scope: None,
        is_enabled: true,
        is_regex: true,
        timeout_millisecond: 3000,
        order: 0,
    }
}

#[test]
fn regex_replace_applies_to_content() {
    // Legado getContent(): isRegex=true → regex.replaceAll(pattern, replacement)
    let processor = ContentProcessor::new(vec![rule(1, r"\s+", " ")]);
    let out = processor.process_content("hello   world\t!", "book", "https://src.test");
    assert_eq!(out, "hello world !");
}

#[test]
fn string_replace_applies_when_is_regex_false() {
    // Legado: isRegex=false → string replace (all occurrences)
    let mut r = rule(1, "广告", "");
    r.is_regex = false;
    let processor = ContentProcessor::new(vec![r]);
    let out = processor.process_content("正文广告内容广告结尾", "book", "https://src.test");
    assert_eq!(out, "正文内容结尾");
}

#[test]
fn empty_pattern_is_skipped_not_error() {
    // Legado getContent(): `if (item.pattern.isEmpty()) return@forEach`
    let r = rule(1, "", "X");
    let processor = ContentProcessor::new(vec![r]);
    let out = processor.process_content("unchanged", "book", "https://src.test");
    assert_eq!(out, "unchanged");
}

#[test]
fn invalid_regex_is_skipped_not_panic() {
    // Legado isValid() rejects bad regex; getContent() catches exceptions per-rule.
    // Rust 侧跳过无效正则,不 panic,其他规则继续执行。
    let bad = rule(1, r"[unclosed", "X");
    let good = rule(2, r"foo", "bar");
    let processor = ContentProcessor::new(vec![bad, good]);
    let out = processor.process_content("foo baz", "book", "https://src.test");
    assert_eq!(out, "bar baz");
}

#[test]
fn rules_apply_in_order_ascending() {
    // Legado: `getContentReplaceRules().forEach` 已按 order 排序;小的先执行。
    let mut first = rule(1, "A", "B");
    first.order = 0;
    let mut second = rule(2, "B", "C");
    second.order = 10;
    // 传入时故意倒序,处理器必须按 order 排序后执行:A→B, B→C = "C"
    let processor = ContentProcessor::new(vec![second, first]);
    let out = processor.process_content("A", "book", "https://src.test");
    assert_eq!(out, "C");
}

#[test]
fn scope_match_includes_book_name_applies() {
    // Legado findEnabledByContentScope: scope LIKE '%name%' → 命中
    let mut r = rule(1, "X", "Y");
    r.scope = Some("斗破苍穹".to_string());
    let processor = ContentProcessor::new(vec![r]);
    let out = processor.process_content("X", "斗破苍穹", "https://src.test");
    assert_eq!(out, "Y");
}

#[test]
fn scope_no_match_skips_rule() {
    // scope 不包含 book_name 也不包含 book_origin → 不应用
    let mut r = rule(1, "X", "Y");
    r.scope = Some("其他书".to_string());
    let processor = ContentProcessor::new(vec![r]);
    let out = processor.process_content("X", "斗破苍穹", "https://src.test");
    assert_eq!(out, "X");
}

#[test]
fn scope_empty_matches_all() {
    // Legado: `scope is null or scope = ''` → 匹配全部
    let r = rule(1, "X", "Y");
    let processor = ContentProcessor::new(vec![r]);
    let out = processor.process_content("X", "任意书", "https://any.test");
    assert_eq!(out, "Y");
}

#[test]
fn exclude_scope_excludes_rule() {
    // Legado: excludeScope LIKE '%origin%' → 排除
    let mut r = rule(1, "X", "Y");
    r.exclude_scope = Some("https://blocked.test".to_string());
    let processor = ContentProcessor::new(vec![r]);
    let out = processor.process_content("X", "book", "https://blocked.test");
    assert_eq!(out, "X");
}

#[test]
fn disabled_rule_skipped() {
    let mut r = rule(1, "X", "Y");
    r.is_enabled = false;
    let processor = ContentProcessor::new(vec![r]);
    let out = processor.process_content("X", "book", "https://src.test");
    assert_eq!(out, "X");
}

#[test]
fn scope_content_false_skipped_for_content() {
    // Legado findEnabledByContentScope: scopeContent = 1 才入选
    let mut r = rule(1, "X", "Y");
    r.scope_content = false;
    r.scope_title = true;
    let processor = ContentProcessor::new(vec![r]);
    let out = processor.process_content("X", "book", "https://src.test");
    assert_eq!(out, "X");
}

#[test]
fn process_title_only_applies_scope_title_rules() {
    // Legado findEnabledByTitleScope: scopeTitle = 1 才入选
    let mut title_rule = rule(1, "第", "第");
    title_rule.scope_title = true;
    title_rule.scope_content = false;
    title_rule.pattern = r"标题".to_string();
    title_rule.replacement = "TITLE".to_string();

    let mut content_rule = rule(2, "正文", "BODY");
    content_rule.scope_title = false;
    content_rule.scope_content = true;

    let processor = ContentProcessor::new(vec![title_rule, content_rule]);
    // 标题处理:只走 scope_title=true 的规则
    let title_out = processor.process_title("标题X", "book", "https://src.test");
    assert_eq!(title_out, "TITLEX");
    // 正文处理:只走 scope_content=true 的规则(标题规则不作用于正文)
    let content_out = processor.process_content("标题正文", "book", "https://src.test");
    assert_eq!(content_out, "标题BODY");
}

#[test]
fn content_lines_are_trimmed_before_replace_like_legado() {
    // Legado getContent(): `mContent.lines().joinToString("\n") { it.trim() }`
    // 先逐行 trim。trim 前 ^x 不命中(行首是空格);trim 后 ^x 命中第一行行首。
    let r = rule(1, r"^x", "Y");
    let processor = ContentProcessor::new(vec![r]);
    let out = processor.process_content("  x\n  y", "book", "https://src.test");
    assert_eq!(out, "Y\ny");
}

#[test]
fn legado_json_import_then_apply_rules_to_content() {
    // 端到端:Legado `ReplaceAnalyzer.jsonToReplaceRules(json)` →
    // `ContentProcessor.getContent()` 逐条 regex 替换。
    // JSON shape 对齐 reader-domain `ReplaceRule` 的 camelCase serde
    // (Legado `ReplaceRule.kt` 字段: id/name/group/pattern/replacement/
    // scope/scopeTitle/scopeContent/excludeScope/isEnabled/isRegex/
    // timeoutMillisecond/order)。
    let json = serde_json::json!([
        {
            "id": 1001,
            "name": "去广告",
            "group": "净化",
            "pattern": r"<ad>.*?</ad>",
            "replacement": "",
            "scope": null,
            "scopeTitle": false,
            "scopeContent": true,
            "excludeScope": null,
            "isEnabled": true,
            "isRegex": true,
            "timeoutMillisecond": 3000,
            "order": 1
        },
        {
            "id": 1002,
            "name": "繁简修正",
            "group": "净化",
            "pattern": "後",
            "replacement": "后",
            "scope": null,
            "scopeTitle": false,
            "scopeContent": true,
            "excludeScope": null,
            "isEnabled": true,
            "isRegex": false,
            "timeoutMillisecond": 3000,
            "order": 2
        }
    ]);

    // Legado jsonToReplaceRules: JSON array → Vec<ReplaceRule>
    let rules: Vec<ReplaceRule> =
        serde_json::from_value(json).expect("Legado replace-rule JSON must deserialize");

    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].id, 1001);
    assert_eq!(rules[0].name, "去广告");
    assert_eq!(rules[0].pattern, r"<ad>.*?</ad>");
    assert!(rules[0].is_regex);
    assert!(!rules[1].is_regex); // plain-text replace
    assert_eq!(rules[1].replacement, "后");

    // ContentProcessor::new(rules) → process_content
    let processor = ContentProcessor::new(rules);
    let content = "<ad>spam</ad>这是後面的文字";
    let out = processor.process_content(content, "mybook", "https://src.test");

    // regex rule strips <ad>…</ad>; then plain-text rule swaps 後→后.
    assert_eq!(out, "这是后面的文字");
}

#[test]
fn legado_json_import_respects_scope_filtering() {
    // 同一组规则,对不同书源产生不同结果:scope 匹配才执行替换。
    let json = serde_json::json!([
        {
            "id": 2001,
            "name": "scoped-ad-removal",
            "pattern": r"\[AD\]",
            "replacement": "",
            "scope": "targetBook",
            "scopeTitle": false,
            "scopeContent": true,
            "isEnabled": true,
            "isRegex": true,
            "timeoutMillisecond": 3000,
            "order": 0
        }
    ]);
    let rules: Vec<ReplaceRule> =
        serde_json::from_value(json).expect("Legado replace-rule JSON must deserialize");

    let processor = ContentProcessor::new(rules);

    // 匹配 scope:书名含 "targetBook" → 规则生效,[AD] 被移除。
    let matched = processor.process_content("hello [AD] world", "targetBook", "https://src.test");
    assert_eq!(matched, "hello  world");

    // 不匹配 scope:书名不含 "targetBook" → 规则跳过,[AD] 保留。
    let unmatched = processor.process_content("hello [AD] world", "otherBook", "https://src.test");
    assert_eq!(unmatched, "hello [AD] world");
}

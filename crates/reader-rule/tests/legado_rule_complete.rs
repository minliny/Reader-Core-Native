//! Tests for `auto_complete_rule` — 1:1 对照 Legado `RuleComplete.kt`。
//!
//! Legado `RuleComplete.autoComplete(rules, preRule, type)` 在书源编辑时被调用
//! (BookSourceEditActivity),对省略尾操作符的简单规则自动补全:
//!   - type=1(文字):补 `@text`(XPath 补 `//text()`),并修正 `img@text` → `img@alt`
//!   - type=2(链接):补 `@href`(XPath 补 `//@href`)
//!   - type=3(图片):补 `@src`(XPath 补 `//@src`)
//!
//! 复杂规则(`{{}}` / `@js:` / `<js>` / `@Json:` / `$.` / `:` 开头 / `##` 开头)不补全。
//! 尾部 `##regex` 或 `,{params}` 分离后只补全主体。

use reader_rule::auto_complete_rule;

#[test]
fn completes_text_rule_with_at_text_suffix() {
    // `div.class&&` -> `div.class@text&&`(type=1,文字)
    assert_eq!(
        auto_complete_rule("div.class&&", None, 1),
        "div.class@text&&"
    );
}

#[test]
fn completes_text_rule_without_separator() {
    // `div.class`(无分隔符) -> `div.class@text`(行尾补全)
    assert_eq!(auto_complete_rule("div.class", None, 1), "div.class@text");
}

#[test]
fn completes_link_rule_with_at_href() {
    // `div.class&&` -> `div.class@href&&`(type=2,链接)
    assert_eq!(
        auto_complete_rule("div.class&&", None, 2),
        "div.class@href&&"
    );
}

#[test]
fn completes_image_rule_with_at_src() {
    // `div.class&&` -> `div.class@src&&`(type=3,图片)
    assert_eq!(
        auto_complete_rule("div.class&&", None, 3),
        "div.class@src&&"
    );
}

#[test]
fn does_not_complete_when_extraction_already_present() {
    // `div.class@text&&` 已有 @text 抽取,不补全
    assert_eq!(
        auto_complete_rule("div.class@text&&", None, 1),
        "div.class@text&&"
    );
    // `div.class@href&&` 已有 @href 抽取
    assert_eq!(
        auto_complete_rule("div.class@href&&", None, 2),
        "div.class@href&&"
    );
    // `div.class@src&&` 已有 @src 抽取
    assert_eq!(
        auto_complete_rule("div.class@src&&", None, 3),
        "div.class@src&&"
    );
}

#[test]
fn does_not_complete_template_rules() {
    // `{{xxx}}` 含模板,不补全(notComplete)
    assert_eq!(auto_complete_rule("{{xxx}}", None, 1), "{{xxx}}");
    assert_eq!(
        auto_complete_rule("{{baseUrl}}/#dir", None, 1),
        "{{baseUrl}}/#dir"
    );
}

#[test]
fn does_not_complete_js_rules() {
    assert_eq!(auto_complete_rule("@js:foo", None, 1), "@js:foo");
    assert_eq!(auto_complete_rule("<js>foo</js>", None, 1), "<js>foo</js>");
}

#[test]
fn does_not_complete_jsonpath_rules() {
    // `$.` 开头的 JSONPath 不补全
    assert_eq!(auto_complete_rule("$.title", None, 1), "$.title");
    assert_eq!(auto_complete_rule("$.books[*]", None, 1), "$.books[*]");
}

#[test]
fn does_not_complete_colon_prefixed_rules() {
    // `:` 开头不补全
    assert_eq!(auto_complete_rule(":div.class", None, 1), ":div.class");
}

#[test]
fn does_not_complete_hash_prefixed_rules() {
    // `##` 开头不补全
    assert_eq!(
        auto_complete_rule("##regex##replacement", None, 1),
        "##regex##replacement"
    );
}

#[test]
fn preserves_tail_regex_suffix() {
    // 尾部 `##regex` 分离后只补全主体
    // `div.class##https` -> `div.class@text##https`
    assert_eq!(
        auto_complete_rule("div.class##https", None, 1),
        "div.class@text##https"
    );
    // `div.class&&##regex` -> `div.class@text&&##regex`
    assert_eq!(
        auto_complete_rule("div.class&&##regex", None, 1),
        "div.class@text&&##regex"
    );
}

#[test]
fn preserves_tail_params_suffix() {
    // 尾部 `,{params}` 分离后只补全主体
    assert_eq!(
        auto_complete_rule("div.class,{\"json\":1}", None, 1),
        "div.class@text,{\"json\":1}"
    );
}

#[test]
fn fix_img_info_corrects_img_at_text_to_alt() {
    // `img@text`(无分隔符) -> `img@alt`(type=1,fixImgInfo)
    assert_eq!(auto_complete_rule("img@text", None, 1), "img@alt");
}

#[test]
fn fix_img_info_corrects_img_at_text_with_separator() {
    // `img@text&&` 的处理链(对齐任务约定:行尾空段不补):
    // 1. needComplete: 段 `img@text` 已有 @text 不补;`&&` 后空段不补
    //    -> `img@text&&`
    // 2. fixImgInfo: `img@text&&` -> `img@alt&&`
    // 最终: `img@alt&&`
    assert_eq!(auto_complete_rule("img@text&&", None, 1), "img@alt&&");
}

#[test]
fn fix_img_info_with_class_at_group() {
    // `img.cover@text&&` -> `img.cover@alt&&`
    // (at group = `.cover`,fixImgInfo 保留 at)
    assert_eq!(
        auto_complete_rule("img.cover@text&&", None, 1),
        "img.cover@alt&&"
    );
}

#[test]
fn fix_img_info_with_attribute_at_group() {
    // `img[@src]@text&&` -> `img[@src]@alt&&`
    assert_eq!(
        auto_complete_rule("img[@src]@text&&", None, 1),
        "img[@src]@alt&&"
    );
}

#[test]
fn fix_img_info_only_applies_to_type_1() {
    // type=2(链接)不应用 fixImgInfo
    // `img@text` -> needComplete 不补(已有 @text) -> `img@text`
    assert_eq!(auto_complete_rule("img@text", None, 2), "img@text");
    // type=3(图片)不应用 fixImgInfo
    assert_eq!(auto_complete_rule("img@text", None, 3), "img@text");
}

#[test]
fn fix_img_info_requires_valid_prefix() {
    // `divimg@text&&`:`img` 前是 `v`,不是合法前缀,fixImgInfo 不应用
    // needComplete: 段 `divimg@text` 已有 @text,不补;`&&` 后空段不补
    // -> `divimg@text&&`
    assert_eq!(
        auto_complete_rule("divimg@text&&", None, 1),
        "divimg@text&&"
    );
}

#[test]
fn fix_img_info_with_tag_prefix() {
    // `tag.img@text&&`:`img` 前是 `tag.`,合法前缀,fixImgInfo 应用
    // -> `tag.img@alt&&`
    assert_eq!(
        auto_complete_rule("tag.img@text&&", None, 1),
        "tag.img@alt&&"
    );
}

#[test]
fn completes_xpath_text_rule() {
    // XPath 规则(以 `//` 开头)补 `//text()`
    // `//div` -> `//div//text()`
    assert_eq!(auto_complete_rule("//div", None, 1), "//div//text()");
}

#[test]
fn completes_xpath_link_rule() {
    // `//div/@href` 已有抽取,不补
    assert_eq!(auto_complete_rule("//div/@href", None, 2), "//div/@href");
    // `//div` -> `//div//@href`
    assert_eq!(auto_complete_rule("//div", None, 2), "//div//@href");
}

#[test]
fn completes_xpath_image_rule() {
    // `//div` -> `//div//@src`(type=3)
    assert_eq!(auto_complete_rule("//div", None, 3), "//div//@src");
}

#[test]
fn completes_multi_segment_rule() {
    // `div.class&&tag.li` -> `div.class@text&&tag.li@text`(两段都补)
    assert_eq!(
        auto_complete_rule("div.class&&tag.li", None, 1),
        "div.class@text&&tag.li@text"
    );
}

#[test]
fn completes_or_fallback_rule() {
    // `div.a||div.b` -> `div.a@text||div.b@text`
    assert_eq!(
        auto_complete_rule("div.a||div.b", None, 1),
        "div.a@text||div.b@text"
    );
}

#[test]
fn completes_parallel_rule() {
    // `div.a%%div.b` -> `div.a@text%%div.b@text`
    assert_eq!(
        auto_complete_rule("div.a%%div.b", None, 1),
        "div.a@text%%div.b@text"
    );
}

#[test]
fn does_not_complete_when_pre_rule_is_complex() {
    // preRule 含 `{{}}`,不补全
    assert_eq!(
        auto_complete_rule("div.class&&", Some("{{baseUrl}}"), 1),
        "div.class&&"
    );
    // preRule 含 `@js:`,不补全
    assert_eq!(
        auto_complete_rule("div.class&&", Some("@js:foo"), 1),
        "div.class&&"
    );
}

#[test]
fn completes_when_pre_rule_is_simple() {
    // preRule 是简单规则,正常补全
    assert_eq!(
        auto_complete_rule("div.class&&", Some("class.item"), 1),
        "div.class@text&&"
    );
}

#[test]
fn returns_unknown_type_unchanged() {
    // type=99 不是 1/2/3,返回原规则
    assert_eq!(auto_complete_rule("div.class&&", None, 99), "div.class&&");
}

#[test]
fn returns_empty_rule_unchanged() {
    assert_eq!(auto_complete_rule("", None, 1), "");
}

#[test]
fn does_not_complete_text_with_parens_extraction() {
    // `div.class@text()` 已有 `text()` 抽取,不补全
    assert_eq!(
        auto_complete_rule("div.class@text()&&", None, 1),
        "div.class@text()&&"
    );
}

#[test]
fn does_not_misidentify_partial_keyword() {
    // `divtext&&`:`text` 前是 `v`,不是合法前缀,不视为已有抽取
    // -> `divtext@text&&`(段 `divtext` 补 `@text`,行尾空段不补)
    assert_eq!(auto_complete_rule("divtext&&", None, 1), "divtext@text&&");
}

#[test]
fn completes_bare_text_keyword_at_start() {
    // `text` 是裸抽取关键字(起始 + 关键字),视为已有抽取,不补
    assert_eq!(auto_complete_rule("text", None, 1), "text");
    // `text&&` -> `text&&`(段 `text` 已有抽取不补,行尾空段不补)
    assert_eq!(auto_complete_rule("text&&", None, 1), "text&&");
}

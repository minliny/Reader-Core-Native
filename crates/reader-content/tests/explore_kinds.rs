//! Tests for `parse_explore_kinds` — Legado `exploreUrl` field parsing.
//!
//! Mirrors Legado `BookSourceExtensions.kt:44 getExploreKinds`:
//! - JSON array form: `[{"title":"...","url":"..."}]`
//! - Plain text form: `"名称::url\n名称::url"` (split by `&&` or newline)
//! - `@js:`/`<js>` form is handled by `RemoteContentPipeline::parse_explore_kinds_with_js`
//!   (covered separately; here we assert the standalone parser returns empty
//!   for JS-wrapped inputs so the pipeline path is the single source of truth).

use reader_content::{parse_explore_kinds, ExploreKind};

#[test]
fn empty_explore_url_yields_no_kinds() {
    assert!(parse_explore_kinds("").is_empty());
    assert!(parse_explore_kinds("   \n\t  ").is_empty());
}

#[test]
fn json_array_form_parses_title_and_url() {
    let raw = r#"[
        {"title":"玄幻","url":"https://example.com/xuanhuan"},
        {"title":"都市","url":"https://example.com/dushi"}
    ]"#;
    let kinds = parse_explore_kinds(raw);
    assert_eq!(kinds.len(), 2);
    assert_eq!(
        kinds[0],
        ExploreKind {
            title: "玄幻".to_string(),
            url: Some("https://example.com/xuanhuan".to_string())
        }
    );
    assert_eq!(
        kinds[1],
        ExploreKind {
            title: "都市".to_string(),
            url: Some("https://example.com/dushi".to_string())
        }
    );
}

#[test]
fn json_array_accepts_name_field_as_title_alias() {
    // Legado sources sometimes use `name` instead of `title`.
    let raw = r#"[{"name":"玄幻","url":"https://example.com/x"}]"#;
    let kinds = parse_explore_kinds(raw);
    assert_eq!(kinds.len(), 1);
    assert_eq!(kinds[0].title, "玄幻");
    assert_eq!(kinds[0].url.as_deref(), Some("https://example.com/x"));
}

#[test]
fn json_array_recurses_into_children() {
    // Legado `ExploreKind` supports nested `children` for grouped categories.
    let raw = r#"[
        {"title":"男生","children":[
            {"title":"玄幻","url":"https://example.com/x"},
            {"title":"都市","url":"https://example.com/d"}
        ]},
        {"title":"女生","children":[
            {"title":"言情","url":"https://example.com/y"}
        ]}
    ]"#;
    let kinds = parse_explore_kinds(raw);
    assert_eq!(kinds.len(), 3);
    assert_eq!(kinds[0].title, "玄幻");
    assert_eq!(kinds[1].title, "都市");
    assert_eq!(kinds[2].title, "言情");
}

#[test]
fn plain_text_form_parses_double_colon_separator() {
    let raw = "玄幻::https://example.com/xuanhuan\n都市::https://example.com/dushi";
    let kinds = parse_explore_kinds(raw);
    assert_eq!(kinds.len(), 2);
    assert_eq!(kinds[0].title, "玄幻");
    assert_eq!(
        kinds[0].url.as_deref(),
        Some("https://example.com/xuanhuan")
    );
    assert_eq!(kinds[1].title, "都市");
    assert_eq!(kinds[1].url.as_deref(), Some("https://example.com/dushi"));
}

#[test]
fn plain_text_form_parses_amp_amp_separator() {
    let raw = "玄幻::https://example.com/x&&都市::https://example.com/d";
    let kinds = parse_explore_kinds(raw);
    assert_eq!(kinds.len(), 2);
    assert_eq!(kinds[0].title, "玄幻");
    assert_eq!(kinds[1].title, "都市");
}

#[test]
fn plain_text_form_handles_mixed_separators() {
    // Legado allows `&&` and newline as interchangeable separators.
    let raw =
        "玄幻::https://example.com/x\n都市::https://example.com/d&&武侠::https://example.com/w";
    let kinds = parse_explore_kinds(raw);
    assert_eq!(kinds.len(), 3);
    assert_eq!(kinds[0].title, "玄幻");
    assert_eq!(kinds[1].title, "都市");
    assert_eq!(kinds[2].title, "武侠");
}

#[test]
fn plain_text_form_skips_entries_without_title() {
    let raw = "::https://example.com/no-title\n玄幻::https://example.com/x";
    let kinds = parse_explore_kinds(raw);
    assert_eq!(kinds.len(), 1);
    assert_eq!(kinds[0].title, "玄幻");
}

#[test]
fn plain_text_form_entry_without_url_yields_none_url() {
    // A category header with no URL (rare but valid in Legado's UI grouping).
    let raw = "男生\n玄幻::https://example.com/x";
    let kinds = parse_explore_kinds(raw);
    assert_eq!(kinds.len(), 2);
    assert_eq!(kinds[0].title, "男生");
    assert_eq!(kinds[0].url, None);
    assert_eq!(kinds[1].title, "玄幻");
    assert_eq!(kinds[1].url.as_deref(), Some("https://example.com/x"));
}

#[test]
fn js_wrapped_form_returns_empty_from_standalone_parser() {
    // The standalone parser deliberately returns empty for JS-wrapped inputs;
    // the JS-aware path lives on `RemoteContentPipeline::parse_explore_kinds_with_js`.
    assert!(parse_explore_kinds("@js:return [{title:'x',url:'y'}]").is_empty());
    assert!(parse_explore_kinds("<js>return [{title:'x',url:'y'}]</js>").is_empty());
}

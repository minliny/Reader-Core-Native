//! Legado `ContentProcessor.kt:91` getContent() — 替换规则管线 Rust 落地。
//!
//! 对照 Legado `data/entities/ReplaceRule.kt` 字段语义 +
//! `ContentProcessor.getContent()` 的逐条替换流程：
//!   1. 繁简转换（`chineseConverter` 分支，ContentProcessor.kt:135-145）
//!   2. 逐行 trim（`mContent.lines().joinToString("\n") { it.trim() }`）
//!   3. 替换规则：按 scope 匹配、按 order 排序、逐条 regex/字符串替换
//!
//! 证据级别：crate test（章程 §9 第 5 问）。

use reader_domain::{
    replace_rule_matches_scope, replace_rule_matches_target, ReplaceRule,
    ReplaceRuleEvaluationContext, ReplaceRuleTarget,
};

use crate::chinese::{convert_chinese, ChineseConverterType};

/// Legado `ContentProcessor` — 持有替换规则集合 + 繁简转换配置。
///
/// 对照 Legado `app/src/main/java/io/legado/app/help/book/ContentProcessor.kt`。
/// `new(replace_rules)` 构造；`with_chinese_converter` 配置繁简转换；
/// `process_content` / `process_title` 执行 Legado getContent() 替换流程。
#[derive(Debug, Clone)]
pub struct ContentProcessor {
    replace_rules: Vec<ReplaceRule>,
    chinese_converter_type: ChineseConverterType,
}

impl Default for ContentProcessor {
    fn default() -> Self {
        Self {
            replace_rules: Vec::new(),
            chinese_converter_type: ChineseConverterType::None,
        }
    }
}

impl ContentProcessor {
    /// 构造一个带替换规则的处理器，繁简转换默认 `None`。
    /// 对照 Legado `ContentProcessor(bookSource)` — 从 bookSource 读取替换规则。
    pub fn new(replace_rules: Vec<ReplaceRule>) -> Self {
        Self {
            replace_rules,
            chinese_converter_type: ChineseConverterType::None,
        }
    }

    /// 设置繁简转换类型（Legado `AppConfig.chineseConverterType`）。
    /// Builder 风格，对照 `RemoteContentPipeline::with_chinese_converter`。
    pub fn with_chinese_converter(mut self, converter: ChineseConverterType) -> Self {
        self.chinese_converter_type = converter;
        self
    }

    /// 处理正文：繁简转换 → 逐行 trim → 逐条替换规则（scopeContent=true）。
    /// 对照 Legado `ContentProcessor.getContent()` 的 useReplace 分支。
    pub fn process_content(&self, content: &str, book_name: &str, book_origin: &str) -> String {
        let converted = convert_chinese(content, self.chinese_converter_type);
        let trimmed = trim_lines(&converted);
        self.apply_rules(&trimmed, book_name, book_origin, ReplaceRuleTarget::Content)
    }

    /// 处理标题：繁简转换 → 逐条替换规则（scopeTitle=true）。
    /// 对照 Legado `ContentProcessor.getTitleReplaceRules()`。
    pub fn process_title(&self, title: &str, book_name: &str, book_origin: &str) -> String {
        let converted = convert_chinese(title, self.chinese_converter_type);
        self.apply_rules(&converted, book_name, book_origin, ReplaceRuleTarget::Title)
    }

    /// 逐条执行替换规则。对照 Legado `getContent()`:
    ///   - 按 order 升序排序
    ///   - 跳过 is_enabled=false
    ///   - 跳过 pattern 为空
    ///   - 跳过 scope 不匹配 / excludeScope 命中
    ///   - 跳过 scopeTitle/scopeContent 不匹配目标
    ///   - is_regex=true 用 regex 替换；false 用字符串替换
    ///   - 无效正则跳过，不 panic
    fn apply_rules(
        &self,
        input: &str,
        book_name: &str,
        book_origin: &str,
        target: ReplaceRuleTarget,
    ) -> String {
        let ctx = ReplaceRuleEvaluationContext {
            book_title: book_name.to_string(),
            source_name: String::new(),
            source_url: book_origin.to_string(),
        };
        let mut sorted: Vec<&ReplaceRule> = self.replace_rules.iter().collect();
        sorted.sort_by_key(|r| r.order);
        let mut result = input.to_string();
        for rule in sorted {
            if !rule.is_enabled {
                continue;
            }
            if rule.pattern.is_empty() {
                continue;
            }
            if !replace_rule_matches_target(rule, target) {
                continue;
            }
            if !replace_rule_matches_scope(rule, &ctx) {
                continue;
            }
            result = apply_single_rule(rule, &result);
        }
        result
    }
}

/// 逐行 trim 后用 `\n` 重新拼接。对照 Legado
/// `mContent.lines().joinToString("\n") { it.trim() }`。
fn trim_lines(content: &str) -> String {
    content
        .lines()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 执行单条替换规则。is_regex=true 用 regex；false 用字符串替换。
/// 无效正则跳过（Legado `isValid()` 拒绝 + getContent 捕获异常）。
fn apply_single_rule(rule: &ReplaceRule, input: &str) -> String {
    if rule.is_regex {
        match regex::Regex::new(&rule.pattern) {
            Ok(re) => re.replace_all(input, rule.replacement.as_str()).to_string(),
            Err(_) => input.to_string(),
        }
    } else {
        input.replace(&rule.pattern, &rule.replacement)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn default_has_no_rules_and_no_conversion() {
        let proc = ContentProcessor::default();
        let out = proc.process_content("hello world", "book", "https://src.test");
        assert_eq!(out, "hello world");
    }

    #[test]
    fn with_chinese_converter_applies_t2s() {
        let proc = ContentProcessor::default().with_chinese_converter(ChineseConverterType::T2S);
        let out = proc.process_content("測試", "book", "https://src.test");
        assert_eq!(out, "测试");
    }
}

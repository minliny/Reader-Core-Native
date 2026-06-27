//! Non-JS rule execution primitives for Reader-Core.
//!
//! This crate owns the first native rule semantics before the protocol/runtime
//! layer is ready. The public API is intentionally local to rule execution:
//! callers provide source text and a list of rule steps, and receive a flat list
//! of string results.

use regex::{Regex, RegexBuilder};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value as JsonValue;
use std::error::Error;
use std::fmt;
use sxd_xpath::{Context, Factory, Value as XPathValue};

pub type RuleResult<T> = Result<T, RuleError>;

#[derive(Debug, Default, Clone, Copy)]
pub struct RuleEngine;

impl RuleEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn execute_step(&self, input: &str, step: &RuleStep) -> RuleResult<RuleOutput> {
        Ok(RuleOutput::new(apply_step(input, step)?))
    }

    pub fn execute_chain(&self, input: &str, steps: &[RuleStep]) -> RuleResult<RuleOutput> {
        let mut values = vec![input.to_string()];

        for (index, step) in steps.iter().enumerate() {
            let mut next = Vec::new();

            if values.is_empty() {
                // Fallback steps can seed new values when the chain has no
                // prior results, letting downstream steps continue instead of
                // short-circuiting on empty.
                if let RuleStep::Fallback(rule) = step {
                    next = rule.values.clone();
                }
            } else {
                for value in &values {
                    match apply_step(value, step) {
                        Ok(mut results) => next.append(&mut results),
                        Err(source) => {
                            return Err(RuleError::ChainStepFailed {
                                index,
                                source: Box::new(source),
                            });
                        }
                    }
                }
            }

            values = next;
            if values.is_empty() {
                // Continue only if a subsequent Fallback step can recover the
                // chain; otherwise short-circuit to avoid needless iteration.
                let has_fallback = steps[index + 1..]
                    .iter()
                    .any(|step| matches!(step, RuleStep::Fallback(_)));
                if !has_fallback {
                    break;
                }
            }
        }

        Ok(RuleOutput::new(values))
    }

    pub fn execute_legado_css(&self, input: &str, rule: &str) -> RuleResult<RuleOutput> {
        let rule = LegadoCssRule::parse(rule)?;
        self.execute_legado_css_rule(input, &rule)
    }

    pub fn execute_optional_legado_css(
        &self,
        input: &str,
        rule: Option<&str>,
    ) -> RuleResult<RuleOutput> {
        let Some(rule) = rule else {
            return Ok(RuleOutput::new(Vec::new()));
        };

        self.execute_legado_css(input, rule)
    }

    pub fn execute_legado_css_rule(
        &self,
        input: &str,
        rule: &LegadoCssRule,
    ) -> RuleResult<RuleOutput> {
        Ok(RuleOutput::new(apply_legado_css(input, rule)?))
    }

    /// Execute a raw Legado rule string with full prefix dispatch.
    ///
    /// Mirrors `AnalyzeRule.SourceRule.init` (lines 545-578): the rule mode is
    /// detected from the string prefix and the body is routed to the matching
    /// engine (CSS / XPath / JSONPath). `@put`/`@get`, `##regex##replacement`,
    /// `{{...}}` and `<js>`/`@js:` handling are layered on top of this.
    ///
    /// Pipeline order mirrors Legado:
    /// 1. Mode detection on the raw rule text (SourceRule.init 546-578).
    /// 2. `@put:{...}` extraction → `scope` (splitPutRule 408-431).
    /// 3. `@get:{key}` substitution from `scope` (makeUpRule getRuleType 698).
    /// 4. `{{expr}}` JS template substitution (makeUpRule jsRuleType 606-609,
    ///    AppPattern.EXP_PATTERN). If `js` is None and templates are present,
    ///    returns empty output (graceful degradation — Legado catches the JS
    ///    eval failure at a higher level).
    /// 5. `##regex##replacement` suffix parsing (splitRegex 627-654).
    /// 6. `<js>...</js>` / `@js:...` segment splitting (splitSourceRule
    ///    498-518, AppPattern.JS_PATTERN). Non-JS segments execute on the
    ///    previous output; JS segments transform each value with `result`
    ///    bound to the current value.
    /// 7. `##` regex applied to each final output value (replaceRegex 436-460).
    pub fn execute_legado_rule(
        &self,
        input: &str,
        rule: &str,
        scope: &mut dyn RuleVariableScope,
        js: Option<&dyn RuleJsEvaluator>,
    ) -> RuleResult<RuleOutput> {
        let rule = rule.trim();
        if rule.is_empty() {
            return Ok(RuleOutput::new(Vec::new()));
        }
        let (mode, body) = detect_legado_rule_mode(rule);
        // @put extraction must happen before @get substitution (Legado
        // splitPutRule → evalMatcher ordering) so @get:{key} can read pairs
        // stored by an earlier @put:{...} in the same rule.
        let body = extract_put_rules(body, scope);
        let body = substitute_get_rules(&body, scope);
        // {{expr}} templates: if present but no evaluator, degrade gracefully
        // to empty output (mirrors Legado catching JS eval failures).
        if body.contains("{{") && js.is_none() {
            return Ok(RuleOutput::new(Vec::new()));
        }
        let body = substitute_js_templates(&body, js);
        let suffix = LegadoRegexSuffix::parse(&body);
        let segments = split_js_segments(suffix.selector);
        let has_js_segment = segments.iter().any(|(_, is_js)| *is_js);
        if has_js_segment && js.is_none() {
            // <js>/@js: present but no evaluator → cannot transform → empty.
            return Ok(RuleOutput::new(Vec::new()));
        }
        let values = if has_js_segment {
            execute_js_pipeline(self, input, mode, &segments, js.unwrap())?
        } else {
            self.execute_mode(input, mode, suffix.selector)?
                .into_values()
        };
        let values = if suffix.has_regex() {
            values.iter().map(|v| suffix.apply(v)).collect()
        } else {
            values
        };
        Ok(RuleOutput::new(values))
    }

    /// Dispatch a selector to the engine matching `mode`. Mirrors the `when`
    /// block in Legado `getStringList` (lines 206-212) / `getString` (293-304).
    fn execute_mode(
        &self,
        input: &str,
        mode: LegadoRuleMode,
        selector: &str,
    ) -> RuleResult<RuleOutput> {
        match mode {
            LegadoRuleMode::Default => self.execute_legado_css(input, selector),
            LegadoRuleMode::Xpath => {
                let step = RuleStep::XPath(XPathRule::new(selector));
                self.execute_step(input, &step)
            }
            LegadoRuleMode::Json => {
                let step = RuleStep::JsonPath(JsonPathRule::new(selector));
                self.execute_step(input, &step)
            }
            LegadoRuleMode::Regex => self.execute_legado_css(input, selector),
            LegadoRuleMode::Js => Ok(RuleOutput::new(Vec::new())),
        }
    }
}

/// Run a multi-segment Legado rule pipeline that contains JS segments.
///
/// Mirrors Legado `getStringList` (lines 200-223): `result` starts as the
/// input content; each non-JS segment extracts from the previous result, and
/// each JS segment transforms every value with `result` bound to that value
/// (Legado `evalJS`, AnalyzeRule.kt:603-606).
fn execute_js_pipeline(
    engine: &RuleEngine,
    input: &str,
    mode: LegadoRuleMode,
    segments: &[(String, bool)],
    js: &dyn RuleJsEvaluator,
) -> RuleResult<Vec<String>> {
    let mut values: Vec<String> = vec![input.to_string()];
    for (text, is_js) in segments {
        if *is_js {
            let mut next = Vec::new();
            for v in &values {
                // Mirrors Legado: eval failure for one value doesn't abort
                // the whole pipeline; the value is dropped.
                if let Ok(result) = js.eval(text, Some(v)) {
                    next.push(result);
                }
            }
            values = next;
        } else if !text.is_empty() {
            let mut next = Vec::new();
            for v in &values {
                // CSS/XPath parse failures on a sub-segment are non-fatal:
                // drop the value, keep going.
                if let Ok(out) = engine.execute_mode(v, mode, text) {
                    next.extend(out.into_values());
                }
            }
            values = next;
        }
        // Empty non-JS segment: pass through (values unchanged). This lets
        // `<js>expr</js>` with no preceding selector evaluate against the
        // raw input, matching Legado's initial `result = content`.
    }
    Ok(values)
}

/// Legado rule mode detected from the rule-string prefix.
///
/// Mirrors `AnalyzeRule.Mode`: `Default` is CSS, the others route to their
/// dedicated engines. `Regex` / `Js` are set by `splitRegex` / `splitSourceRule`
/// in Legado and are handled by later tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegadoRuleMode {
    Default,
    Xpath,
    Json,
    Regex,
    Js,
}

/// Variable scope for `@put` / `@get`. Implementations store `@put:{...}` pairs
/// (Legado `putMap`) and resolve `@get:{key}` during rule evaluation
/// (`makeUpRule` getRuleType, AnalyzeRule.kt:698).
pub trait RuleVariableScope {
    fn get(&self, key: &str) -> Option<String>;
    fn put(&mut self, key: String, value: String);
    fn entries(&self) -> Vec<(&str, &str)>;
}

/// No-op scope for tests / rules that carry no `@put` / `@get`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopVariableScope;
impl RuleVariableScope for NoopVariableScope {
    fn get(&self, _key: &str) -> Option<String> {
        None
    }
    fn put(&mut self, _key: String, _value: String) {}
    fn entries(&self) -> Vec<(&str, &str)> {
        Vec::new()
    }
}

/// JS evaluator for `{{...}}` and `<js>...</js>` segments.
///
/// `context` is the current extraction result (Legado binds `result` inside
/// `evalJS`). Implementations back this with a sandbox; `reader-rule` itself
/// stays free of any `reader-js` dependency.
pub trait RuleJsEvaluator {
    fn eval(&self, expr: &str, context: Option<&str>) -> Result<String, String>;
}

/// Detect Legado rule mode from the rule-string prefix.
///
/// Returns `(mode, body_with_prefix_stripped)`. Mirrors
/// `AnalyzeRule.SourceRule.init` lines 546-578. `@CSS:`/`@@` switch to CSS
/// mode and strip the prefix; `@XPath:`/`@Json:` strip the prefix and switch
/// mode; `$.`/`$[` and a leading `/` switch to JSON / XPath without stripping.
fn detect_legado_rule_mode(rule: &str) -> (LegadoRuleMode, &str) {
    if let Some(rest) = rule
        .strip_prefix("@CSS:")
        .or_else(|| rule.strip_prefix("@css:"))
    {
        return (LegadoRuleMode::Default, rest);
    }
    if let Some(rest) = rule.strip_prefix("@@") {
        return (LegadoRuleMode::Default, rest);
    }
    if let Some(rest) = rule
        .strip_prefix("@XPath:")
        .or_else(|| rule.strip_prefix("@xpath:"))
    {
        return (LegadoRuleMode::Xpath, rest);
    }
    if let Some(rest) = rule
        .strip_prefix("@Json:")
        .or_else(|| rule.strip_prefix("@json:"))
    {
        return (LegadoRuleMode::Json, rest);
    }
    if rule.starts_with("$.") || rule.starts_with("$[") {
        return (LegadoRuleMode::Json, rule);
    }
    if rule.starts_with('/') {
        return (LegadoRuleMode::Xpath, rule);
    }
    (LegadoRuleMode::Default, rule)
}

/// Parsed `##regex##replacement` suffix from a Legado rule body.
///
/// Mirrors `AnalyzeRule.SourceRule.init` lines 708-718: after mode detection
/// the body is split on `##`. The first part is the selector; subsequent parts
/// configure optional regex replacement applied to each extracted value via
/// `replaceRegex` (lines 436-460).
///
/// `replaceFirst` semantics (Legado 441-452): find the first regex match; if
/// found, return the `replacement` template expanded against the match's
/// capture groups (the rest of the value is discarded); if no match, return
/// the empty string; if the regex fails to compile, return `replacement`
/// verbatim.
///
/// replace-all semantics (Legado 453-459): if the regex compiles, replace
/// every match with `replacement`; if it fails to compile, fall back to
/// literal `str::replace`.
struct LegadoRegexSuffix<'a> {
    /// The selector portion (before any `##`), trimmed.
    selector: &'a str,
    /// The regex pattern (not trimmed). `None` if no `##` present.
    replace_regex: Option<&'a str>,
    /// Replacement template (supports `$1`/`$2`/`${name}` capture expansion).
    replacement: String,
    /// If true, only the first match is considered (Legado `replaceFirst`).
    replace_first: bool,
}

impl<'a> LegadoRegexSuffix<'a> {
    fn parse(body: &'a str) -> Self {
        let mut parts = body.split("##");
        let selector = parts.next().unwrap_or("").trim();
        let replace_regex = parts.next();
        let replacement = parts.next().unwrap_or("").to_string();
        let replace_first = parts.next().is_some();
        Self {
            selector,
            replace_regex,
            replacement,
            replace_first,
        }
    }

    fn has_regex(&self) -> bool {
        self.replace_regex.is_some_and(|r| !r.is_empty())
    }

    fn apply(&self, value: &str) -> String {
        let Some(regex_str) = self.replace_regex else {
            return value.to_string();
        };
        if regex_str.is_empty() {
            return value.to_string();
        }
        if self.replace_first {
            match Regex::new(regex_str) {
                Ok(regex) => match regex.captures(value) {
                    Some(caps) => {
                        let mut dst = String::new();
                        caps.expand(self.replacement.as_str(), &mut dst);
                        dst
                    }
                    None => String::new(),
                },
                Err(_) => self.replacement.clone(),
            }
        } else {
            match Regex::new(regex_str) {
                Ok(regex) => regex
                    .replace_all(value, self.replacement.as_str())
                    .into_owned(),
                Err(_) => value.replace(regex_str, self.replacement.as_str()),
            }
        }
    }
}

/// Extract `@put:{...}` directives from a rule body, store the parsed
/// key-value pairs into `scope`, and return the body with all `@put:{...}`
/// segments removed.
///
/// Mirrors Legado `splitPutRule` (AnalyzeRule.kt:408-431) + `putPattern`
/// (line 880, `@put:(\{[^}]+?\})` case-insensitive). The JSON object inside
/// the braces is parsed as a string-keyed map; non-string values are
/// JSON-stringified. Parse failures are silently skipped (Legado logs a
/// warning but continues) — the `@put:{...}` segment is still stripped.
fn extract_put_rules(body: &str, scope: &mut dyn RuleVariableScope) -> String {
    let regex = match Regex::new(r"(?i)@put:(\{[^}]+?\})") {
        Ok(r) => r,
        Err(_) => return body.to_string(),
    };
    let mut cleaned = body.to_string();
    // Collect first, mutate scope after iteration to avoid borrowing issues.
    let mut to_store: Vec<(String, String)> = Vec::new();
    for caps in regex.captures_iter(body) {
        let full = caps.get(0).map(|m| m.as_str()).unwrap_or("");
        let json_str = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        cleaned = cleaned.replace(full, "");
        if let Ok(map) = serde_json::from_str::<serde_json::Map<String, JsonValue>>(json_str) {
            for (k, v) in map {
                let value = match v {
                    JsonValue::String(s) => s,
                    other => other.to_string(),
                };
                to_store.push((k, value));
            }
        }
    }
    for (k, v) in to_store {
        scope.put(k, v);
    }
    cleaned
}

/// Replace `@get:{key}` with the value stored in `scope`. Missing keys
/// resolve to the empty string, matching Legado `get()` (AnalyzeRule.kt:754-769)
/// which falls back to `""` when no variable / book / chapter / source has
/// the key.
///
/// Mirrors the @get branch of Legado `evalPattern` (line 881,
/// `@get:\{[^}]+?\}` case-insensitive) + `makeUpRule` getRuleType handling
/// (line 698-699).
fn substitute_get_rules(body: &str, scope: &dyn RuleVariableScope) -> String {
    let regex = match Regex::new(r"(?i)@get:\{([^}]+?)\}") {
        Ok(r) => r,
        Err(_) => return body.to_string(),
    };
    regex
        .replace_all(body, |caps: &regex::Captures| {
            let key = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            scope.get(key).unwrap_or_default()
        })
        .into_owned()
}

/// Substitute `{{expr}}` JS templates in the rule body.
///
/// Mirrors Legado `makeUpRule` jsRuleType (AnalyzeRule.kt:606-609) +
/// `AppPattern.EXP_PATTERN` (`{{([\w\W]*?)}}`). Each template is evaluated
/// with no `result` context (the expression runs against the scope, not the
/// extracted value) and its string result replaces the template in the body.
///
/// If `js` is None the body is returned unchanged — callers should check for
/// remaining `{{` and degrade gracefully (see `execute_legado_rule`).
fn substitute_js_templates(body: &str, js: Option<&dyn RuleJsEvaluator>) -> String {
    let Some(js) = js else {
        return body.to_string();
    };
    let regex = match Regex::new(r"(?i)\{\{([\w\W]*?)\}\}") {
        Ok(r) => r,
        Err(_) => return body.to_string(),
    };
    regex
        .replace_all(body, |caps: &regex::Captures| {
            let expr = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            js.eval(expr, None).unwrap_or_default()
        })
        .into_owned()
}

/// Split a rule body into alternating non-JS / JS segments.
///
/// Mirrors Legado `splitSourceRule` (AnalyzeRule.kt:498-518) +
/// `AppPattern.JS_PATTERN` (`<js>([\w\W]*?)</js>|@js:([\w\W]*)`).
/// - `<js>expr</js>` captures `expr` as a JS segment (non-greedy).
/// - `@js:expr` captures `expr` as a JS segment (greedy to end of body —
///   matching Legado's `([\\w\\W]*)` semantics; anything after `@js:` is JS).
///
/// Returns a vec of `(text, is_js)` tuples in source order. Non-JS segments
/// may be empty (e.g. when the body starts with `<js>`).
fn split_js_segments(body: &str) -> Vec<(String, bool)> {
    let regex = match Regex::new(r"(?i)<js>([\w\W]*?)</js>|@js:([\w\W]*)") {
        Ok(r) => r,
        Err(_) => return vec![(body.to_string(), false)],
    };
    let mut segments = Vec::new();
    let mut last_end = 0;
    for caps in regex.captures_iter(body) {
        let m = match caps.get(0) {
            Some(m) => m,
            None => continue,
        };
        // Leading non-JS text before this match.
        if m.start() > last_end {
            segments.push((body[last_end..m.start()].to_string(), false));
        } else if m.start() == last_end && last_end == 0 {
            // Body starts with a JS segment — emit an empty non-JS prefix so
            // the pipeline knows there's no preceding selector. This empty
            // segment is a pass-through (values stay as the input).
            segments.push((String::new(), false));
        }
        let js_expr = caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");
        segments.push((js_expr.to_string(), true));
        last_end = m.end();
    }
    if last_end < body.len() {
        segments.push((body[last_end..].to_string(), false));
    }
    if segments.is_empty() {
        segments.push((body.to_string(), false));
    }
    segments
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleOutput {
    values: Vec<String>,
}

impl RuleOutput {
    pub fn new(values: Vec<String>) -> Self {
        Self { values }
    }

    pub fn values(&self) -> &[String] {
        &self.values
    }

    pub fn first(&self) -> Option<&str> {
        self.values.first().map(String::as_str)
    }

    pub fn into_values(self) -> Vec<String> {
        self.values
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleStep {
    RegexExtract(RegexExtractRule),
    RegexReplace(RegexReplaceRule),
    JsonPath(JsonPathRule),
    Css(CssRule),
    XPath(XPathRule),
    /// Passes through non-empty input unchanged. When a chain reaches this
    /// step with no prior results, the configured `values` are emitted so
    /// downstream steps continue with a deterministic default.
    Fallback(FallbackRule),
}

impl RuleStep {
    pub fn regex_capture(pattern: impl Into<String>, group: CaptureGroup) -> Self {
        Self::RegexExtract(RegexExtractRule::all(pattern, group))
    }

    pub fn regex_capture_first(pattern: impl Into<String>, group: CaptureGroup) -> Self {
        Self::RegexExtract(RegexExtractRule::first(pattern, group))
    }

    pub fn regex_captures(pattern: impl Into<String>) -> Self {
        Self::RegexExtract(RegexExtractRule::all(pattern, CaptureGroup::AllGroups))
    }

    pub fn regex_replace(pattern: impl Into<String>, replacement: impl Into<String>) -> Self {
        Self::RegexReplace(RegexReplaceRule::all(pattern, replacement))
    }

    pub fn regex_replace_first(pattern: impl Into<String>, replacement: impl Into<String>) -> Self {
        Self::RegexReplace(RegexReplaceRule::first(pattern, replacement))
    }

    pub fn json_path(path: impl Into<String>) -> Self {
        Self::JsonPath(JsonPathRule::new(path))
    }

    pub fn css_text(selector: impl Into<String>) -> Self {
        Self::Css(CssRule::text(selector))
    }

    pub fn css_attr(selector: impl Into<String>, attr: impl Into<String>) -> Self {
        Self::Css(CssRule::attr(selector, attr))
    }

    pub fn xpath(expression: impl Into<String>) -> Self {
        Self::XPath(XPathRule::new(expression))
    }

    pub fn xpath_with_namespaces<I, P, U>(expression: impl Into<String>, namespaces: I) -> Self
    where
        I: IntoIterator<Item = (P, U)>,
        P: Into<String>,
        U: Into<String>,
    {
        Self::XPath(XPathRule::with_namespaces(expression, namespaces))
    }

    pub fn fallback<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Fallback(FallbackRule::new(values))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexExtractRule {
    pub pattern: String,
    pub group: CaptureGroup,
    pub all: bool,
}

impl RegexExtractRule {
    pub fn all(pattern: impl Into<String>, group: CaptureGroup) -> Self {
        Self {
            pattern: pattern.into(),
            group,
            all: true,
        }
    }

    pub fn first(pattern: impl Into<String>, group: CaptureGroup) -> Self {
        Self {
            pattern: pattern.into(),
            group,
            all: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureGroup {
    WholeMatch,
    AllGroups,
    Index(usize),
    Name(String),
}

impl CaptureGroup {
    pub fn index(index: usize) -> Self {
        Self::Index(index)
    }

    pub fn name(name: impl Into<String>) -> Self {
        Self::Name(name.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegexReplaceRule {
    pub pattern: String,
    pub replacement: String,
    pub all: bool,
}

impl RegexReplaceRule {
    pub fn all(pattern: impl Into<String>, replacement: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            replacement: replacement.into(),
            all: true,
        }
    }

    pub fn first(pattern: impl Into<String>, replacement: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            replacement: replacement.into(),
            all: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonPathRule {
    pub path: String,
}

impl JsonPathRule {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssRule {
    pub selector: String,
    pub extraction: CssExtraction,
}

impl CssRule {
    pub fn text(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            extraction: CssExtraction::Text,
        }
    }

    pub fn attr(selector: impl Into<String>, attr: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            extraction: CssExtraction::Attr(attr.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CssExtraction {
    Text,
    Attr(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegadoCssRule {
    steps: Vec<LegadoCssStep>,
}

impl LegadoCssRule {
    pub fn parse(rule: &str) -> RuleResult<Self> {
        parse_legado_css_rule(rule).map_err(|message| RuleError::LegadoCssSyntax {
            rule: rule.to_string(),
            message,
        })
    }

    pub fn missing() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn steps(&self) -> &[LegadoCssStep] {
        &self.steps
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegadoCssStep {
    Select(String),
    Extract {
        selector: Option<String>,
        extraction: LegadoCssExtraction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegadoCssExtraction {
    Text,
    Html,
    Attr(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XPathRule {
    pub expression: String,
    pub namespaces: Vec<(String, String)>,
}

impl XPathRule {
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            namespaces: Vec::new(),
        }
    }

    pub fn with_namespaces<I, P, U>(expression: impl Into<String>, namespaces: I) -> Self
    where
        I: IntoIterator<Item = (P, U)>,
        P: Into<String>,
        U: Into<String>,
    {
        Self {
            expression: expression.into(),
            namespaces: namespaces
                .into_iter()
                .map(|(prefix, uri)| (prefix.into(), uri.into()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackRule {
    pub values: Vec<String>,
}

impl FallbackRule {
    pub fn new<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            values: values.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleError {
    RegexSyntax {
        pattern: String,
        message: String,
    },
    RegexCaptureGroupMissing {
        pattern: String,
        group: String,
    },
    RegexReplacementCaptureMissing {
        pattern: String,
        group: String,
    },
    JsonParse {
        message: String,
    },
    JsonPathSyntax {
        path: String,
        message: String,
    },
    CssSelectorSyntax {
        selector: String,
        message: String,
    },
    LegadoCssSyntax {
        rule: String,
        message: String,
    },
    XPathInputParse {
        message: String,
    },
    XPathSyntax {
        expression: String,
        message: String,
    },
    XPathEvaluation {
        expression: String,
        message: String,
    },
    ChainStepFailed {
        index: usize,
        source: Box<RuleError>,
    },
}

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleError::RegexSyntax { pattern, message } => {
                write!(f, "invalid regex `{pattern}`: {message}")
            }
            RuleError::RegexCaptureGroupMissing { pattern, group } => {
                write!(
                    f,
                    "regex `{pattern}` does not define capture group `{group}`"
                )
            }
            RuleError::RegexReplacementCaptureMissing { pattern, group } => {
                write!(
                    f,
                    "regex replacement references missing capture group `{group}` in `{pattern}`"
                )
            }
            RuleError::JsonParse { message } => write!(f, "invalid JSON input: {message}"),
            RuleError::JsonPathSyntax { path, message } => {
                write!(f, "invalid JSONPath `{path}`: {message}")
            }
            RuleError::CssSelectorSyntax { selector, message } => {
                write!(f, "invalid CSS selector `{selector}`: {message}")
            }
            RuleError::LegadoCssSyntax { rule, message } => {
                write!(f, "invalid Legado CSS rule `{rule}`: {message}")
            }
            RuleError::XPathInputParse { message } => {
                write!(f, "invalid XML input for XPath: {message}")
            }
            RuleError::XPathSyntax {
                expression,
                message,
            } => {
                write!(f, "invalid XPath `{expression}`: {message}")
            }
            RuleError::XPathEvaluation {
                expression,
                message,
            } => {
                write!(f, "XPath `{expression}` failed: {message}")
            }
            RuleError::ChainStepFailed { index, source } => {
                write!(f, "rule chain step {index} failed: {source}")
            }
        }
    }
}

impl Error for RuleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            RuleError::ChainStepFailed { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonPathSegment {
    Field(String),
    Index(usize),
    /// `[-1]` → `IndexFromEnd(1)`, `[-2]` → `IndexFromEnd(2)`. Resolved against
    /// the array length at evaluation time.
    IndexFromEnd(usize),
    Wildcard,
    /// `..field`, `..*`, or `..[index]` — descend into every object/array and
    /// apply the inner segment at every depth.
    RecursiveDescent(Box<JsonPathSegment>),
    /// `[0,2]` or `['title','name']` — apply each bracket segment to the same
    /// input value and preserve the declared order.
    Union(Vec<JsonPathSegment>),
    /// `[start:end:step]` — Python-style slice. `start`/`end` are optional and
    /// may be negative (counted from the end); `step` defaults to 1 and may be
    /// negative to reverse iteration. Resolved against the array length at
    /// evaluation time.
    Slice(JsonPathSlice),
    /// `[?(@.field == 'value')]`, `[?(@.field >= 1)]`, or
    /// `[?(@.field =~ /value/i)]` — filter array items by a scalar field before
    /// applying subsequent JSONPath segments.
    Filter(JsonPathFilter),
    Function(JsonPathFunction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonPathFunction {
    Length(Option<Vec<JsonPathSegment>>),
    First,
    Last,
    Index(JsonPathFunctionArgument),
    Sum(Vec<JsonPathFunctionArgument>),
    Min(Vec<JsonPathFunctionArgument>),
    Max(Vec<JsonPathFunctionArgument>),
    Avg(Vec<JsonPathFunctionArgument>),
    StdDev(Vec<JsonPathFunctionArgument>),
    Keys,
    Values,
    Concat(Vec<JsonPathFunctionArgument>),
    Append(Vec<JsonPathFunctionArgument>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonPathFunctionArgument {
    Value(JsonValue),
    Path(Vec<JsonPathSegment>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JsonPathSlice {
    start: Option<isize>,
    end: Option<isize>,
    step: Option<isize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonPathFilter {
    Exists(Vec<String>),
    Empty(Vec<String>),
    Compare(JsonPathFilterComparison),
    All(Vec<JsonPathFilter>),
    Any(Vec<JsonPathFilter>),
    Not(Box<JsonPathFilter>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JsonPathFilterComparison {
    left: JsonPathFilterValueRef,
    op: JsonPathFilterOp,
    right: JsonPathFilterOperand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonPathFilterOp {
    Equal,
    NotEqual,
    RegexMatch,
    In,
    NotIn,
    AnyOf,
    NoneOf,
    SubsetOf,
    Size,
    Empty,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonPathFilterLiteral {
    String(String),
    Number(String),
    Bool(bool),
    Null,
    Regex(JsonPathFilterRegex),
    Array(Vec<JsonPathFilterLiteral>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JsonPathFilterRegex {
    pattern: String,
    flags: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonPathFilterOperand {
    Literal(JsonPathFilterLiteral),
    Path(JsonPathFilterValueRef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonPathFilterValueRef {
    Current,
    Path(Vec<String>),
}

fn apply_step(input: &str, step: &RuleStep) -> RuleResult<Vec<String>> {
    match step {
        RuleStep::RegexExtract(rule) => apply_regex_extract(input, rule),
        RuleStep::RegexReplace(rule) => apply_regex_replace(input, rule),
        RuleStep::JsonPath(rule) => apply_json_path(input, rule),
        RuleStep::Css(rule) => apply_css(input, rule),
        RuleStep::XPath(rule) => apply_xpath(input, rule),
        RuleStep::Fallback(rule) => apply_fallback(input, rule),
    }
}

fn apply_fallback(input: &str, rule: &FallbackRule) -> RuleResult<Vec<String>> {
    if input.is_empty() {
        Ok(rule.values.clone())
    } else {
        Ok(vec![input.to_string()])
    }
}

fn apply_regex_extract(input: &str, rule: &RegexExtractRule) -> RuleResult<Vec<String>> {
    let regex = Regex::new(&rule.pattern).map_err(|err| RuleError::RegexSyntax {
        pattern: rule.pattern.clone(),
        message: err.to_string(),
    })?;

    validate_capture_group(&regex, &rule.pattern, &rule.group)?;

    let mut output = Vec::new();
    for captures in regex.captures_iter(input) {
        match &rule.group {
            CaptureGroup::AllGroups => {
                for index in 1..captures.len() {
                    if let Some(value) = captures.get(index) {
                        output.push(value.as_str().to_string());
                    }
                }
            }
            CaptureGroup::WholeMatch => {
                if let Some(value) = captures.get(0) {
                    output.push(value.as_str().to_string());
                }
            }
            CaptureGroup::Index(index) => {
                if let Some(value) = captures.get(*index) {
                    output.push(value.as_str().to_string());
                }
            }
            CaptureGroup::Name(name) => {
                if let Some(value) = captures.name(name) {
                    output.push(value.as_str().to_string());
                }
            }
        }

        if !rule.all {
            break;
        }
    }

    Ok(output)
}

fn apply_regex_replace(input: &str, rule: &RegexReplaceRule) -> RuleResult<Vec<String>> {
    let regex = Regex::new(&rule.pattern).map_err(|err| RuleError::RegexSyntax {
        pattern: rule.pattern.clone(),
        message: err.to_string(),
    })?;

    validate_replacement_captures(&regex, &rule.pattern, &rule.replacement)?;

    let replaced = if rule.all {
        regex.replace_all(input, rule.replacement.as_str())
    } else {
        regex.replacen(input, 1, rule.replacement.as_str())
    };

    Ok(vec![replaced.into_owned()])
}

fn validate_capture_group(regex: &Regex, pattern: &str, group: &CaptureGroup) -> RuleResult<()> {
    match group {
        CaptureGroup::WholeMatch => Ok(()),
        CaptureGroup::AllGroups if regex.captures_len() > 1 => Ok(()),
        CaptureGroup::AllGroups => Err(RuleError::RegexCaptureGroupMissing {
            pattern: pattern.to_string(),
            group: "all".to_string(),
        }),
        CaptureGroup::Index(index) if *index < regex.captures_len() => Ok(()),
        CaptureGroup::Index(index) => Err(RuleError::RegexCaptureGroupMissing {
            pattern: pattern.to_string(),
            group: index.to_string(),
        }),
        CaptureGroup::Name(name) if regex.capture_names().any(|capture| capture == Some(name)) => {
            Ok(())
        }
        CaptureGroup::Name(name) => Err(RuleError::RegexCaptureGroupMissing {
            pattern: pattern.to_string(),
            group: name.clone(),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplacementCaptureRef {
    Index(usize),
    Name(String),
}

fn validate_replacement_captures(
    regex: &Regex,
    pattern: &str,
    replacement: &str,
) -> RuleResult<()> {
    for reference in replacement_capture_refs(replacement) {
        match reference {
            ReplacementCaptureRef::Index(index) if index < regex.captures_len() => {}
            ReplacementCaptureRef::Index(index) => {
                return Err(RuleError::RegexReplacementCaptureMissing {
                    pattern: pattern.to_string(),
                    group: index.to_string(),
                });
            }
            ReplacementCaptureRef::Name(name)
                if regex
                    .capture_names()
                    .any(|capture| capture == Some(name.as_str())) => {}
            ReplacementCaptureRef::Name(name) => {
                return Err(RuleError::RegexReplacementCaptureMissing {
                    pattern: pattern.to_string(),
                    group: name,
                });
            }
        }
    }

    Ok(())
}

fn replacement_capture_refs(replacement: &str) -> Vec<ReplacementCaptureRef> {
    let chars = replacement.chars().collect::<Vec<_>>();
    let mut refs = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index] != '$' {
            index += 1;
            continue;
        }

        index += 1;
        if index >= chars.len() {
            break;
        }
        if chars[index] == '$' {
            index += 1;
            continue;
        }

        if chars[index] == '{' {
            index += 1;
            let start = index;
            while index < chars.len() && chars[index] != '}' {
                index += 1;
            }
            if index >= chars.len() {
                break;
            }

            let token = chars[start..index].iter().collect::<String>();
            if !token.is_empty() {
                refs.push(capture_ref_from_token(&token));
            }
            index += 1;
            continue;
        }

        if chars[index].is_ascii_digit() {
            let start = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            refs.push(ReplacementCaptureRef::Index(
                chars[start..index]
                    .iter()
                    .collect::<String>()
                    .parse::<usize>()
                    .unwrap_or(usize::MAX),
            ));
            continue;
        }

        if is_capture_name_start(chars[index]) {
            let start = index;
            index += 1;
            while index < chars.len() && is_capture_name_continue(chars[index]) {
                index += 1;
            }
            refs.push(ReplacementCaptureRef::Name(
                chars[start..index].iter().collect(),
            ));
            continue;
        }

        index += 1;
    }

    refs
}

fn capture_ref_from_token(token: &str) -> ReplacementCaptureRef {
    token
        .parse::<usize>()
        .map(ReplacementCaptureRef::Index)
        .unwrap_or_else(|_| ReplacementCaptureRef::Name(token.to_string()))
}

fn is_capture_name_start(value: char) -> bool {
    value == '_' || value.is_ascii_alphabetic()
}

fn is_capture_name_continue(value: char) -> bool {
    value == '_' || value.is_ascii_alphanumeric()
}

fn apply_json_path(input: &str, rule: &JsonPathRule) -> RuleResult<Vec<String>> {
    let value: JsonValue = serde_json::from_str(input).map_err(|err| RuleError::JsonParse {
        message: err.to_string(),
    })?;

    evaluate_json_path_expression(&value, &rule.path).map_err(|message| RuleError::JsonPathSyntax {
        path: rule.path.clone(),
        message,
    })
}

fn evaluate_json_path_expression(value: &JsonValue, path: &str) -> Result<Vec<String>, String> {
    if let Some(branches) = split_json_path_top_level_operator(path, "||")? {
        for branch in branches {
            let results = evaluate_json_path_expression(value, branch)?;
            if !results.is_empty() {
                return Ok(results);
            }
        }
        return Ok(Vec::new());
    }

    if let Some(branches) = split_json_path_top_level_operator(path, "%%")? {
        let mut branch_results = Vec::new();
        for branch in branches {
            let results = evaluate_json_path_expression(value, branch)?;
            if !results.is_empty() {
                branch_results.push(results);
            }
        }
        return Ok(zip_json_path_combination_results(branch_results));
    }

    if let Some(branches) = split_json_path_top_level_operator(path, "&&")? {
        let mut output = Vec::new();
        for branch in branches {
            let results = evaluate_json_path_expression(value, branch)?;
            if !results.is_empty() {
                output.extend(results);
            }
        }
        return Ok(output);
    }

    evaluate_json_path_rule(value, path)
}

fn zip_json_path_combination_results(results: Vec<Vec<String>>) -> Vec<String> {
    let Some(first) = results.first() else {
        return Vec::new();
    };

    let mut output = Vec::new();
    for index in 0..first.len() {
        for result in &results {
            if let Some(value) = result.get(index) {
                output.push(value.clone());
            }
        }
    }

    output
}

fn evaluate_json_path_rule(value: &JsonValue, path: &str) -> Result<Vec<String>, String> {
    let segments = parse_json_path(path)?;
    if json_path_segments_contain_function(&segments) {
        return Ok(evaluate_json_path_with_functions(&value, &segments)
            .iter()
            .map(json_value_to_rule_text)
            .collect());
    }

    Ok(evaluate_json_path(&value, &segments)
        .into_iter()
        .map(json_value_to_rule_text)
        .collect())
}

fn split_json_path_top_level_operator<'a>(
    path: &'a str,
    operator: &str,
) -> Result<Option<Vec<&'a str>>, String> {
    let operator_marker = operator
        .chars()
        .next()
        .ok_or_else(|| "JSONPath combination operator must not be empty".to_string())?;
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut branches = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;

    while index < path.len() {
        let value = path[index..]
            .chars()
            .next()
            .ok_or_else(|| "invalid JSONPath fallback expression".to_string())?;

        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            index += value.len_utf8();
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            index += value.len_utf8();
            continue;
        }

        if value == operator_marker
            && path[index..].starts_with(operator)
            && bracket_depth == 0
            && paren_depth == 0
            && brace_depth == 0
        {
            let branch = path[start..index].trim();
            if branch.is_empty() {
                return Err(format!(
                    "empty JSONPath `{operator}` combination branch in `{path}`"
                ));
            }
            branches.push(branch);
            index += operator.len();
            start = index;
            continue;
        }

        match value {
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            '{' if paren_depth > 0 => brace_depth += 1,
            '}' if brace_depth > 0 => brace_depth -= 1,
            _ => {}
        }

        index += value.len_utf8();
    }

    if branches.is_empty() {
        return Ok(None);
    }

    if quote.is_some() || bracket_depth != 0 || paren_depth != 0 || brace_depth != 0 {
        return Err(format!(
            "unterminated JSONPath `{operator}` combination branch in `{path}`"
        ));
    }

    let branch = path[start..].trim();
    if branch.is_empty() {
        return Err(format!(
            "empty JSONPath `{operator}` combination branch in `{path}`"
        ));
    }
    branches.push(branch);

    Ok(Some(branches))
}

fn json_path_segments_contain_function(segments: &[JsonPathSegment]) -> bool {
    segments
        .iter()
        .any(|segment| matches!(segment, JsonPathSegment::Function(_)))
}

fn evaluate_json_path_with_functions(
    root: &JsonValue,
    segments: &[JsonPathSegment],
) -> Vec<JsonValue> {
    let mut current = vec![root.clone()];

    for segment in segments {
        if let JsonPathSegment::Function(function) = segment {
            let input = current.iter().collect::<Vec<_>>();
            current = apply_json_path_function(root, function.clone(), input);
            continue;
        }

        let mut next = Vec::new();
        for value in &current {
            let mut matches = Vec::new();
            apply_json_path_segment(segment, value, &mut matches);
            next.extend(matches.into_iter().cloned());
        }
        current = next;
    }

    current
}

fn parse_json_path(path: &str) -> Result<Vec<JsonPathSegment>, String> {
    let chars = path.chars().collect::<Vec<_>>();
    if chars.first() != Some(&'$') {
        return Err("path must start with `$`".to_string());
    }

    let mut segments = Vec::new();
    let mut index = 1;

    while index < chars.len() {
        match chars[index] {
            '.' => {
                index += 1;
                if index >= chars.len() {
                    return Err("field name expected after `.`".to_string());
                }
                if chars[index] == '.' {
                    segments.push(parse_recursive_descent(&chars, &mut index)?);
                    continue;
                }
                if chars[index] == '*' {
                    segments.push(JsonPathSegment::Wildcard);
                    index += 1;
                    continue;
                }

                let start = index;
                index = json_path_dot_token_end(&chars, index)?;
                if start == index {
                    return Err("field name expected after `.`".to_string());
                }
                let token = chars[start..index].iter().collect::<String>();
                segments.push(parse_json_path_dot_token(&token)?);
            }
            '[' => {
                index += 1;
                let (segment, next_index) = parse_json_path_bracket(&chars, index)?;
                segments.push(segment);
                index = next_index;
            }
            current => {
                return Err(format!("expected `.` or `[` at `{current}`"));
            }
        }
    }

    Ok(segments)
}

fn json_path_dot_token_end(chars: &[char], start: usize) -> Result<usize, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut index = start;

    while index < chars.len() {
        let value = chars[index];
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            index += 1;
            continue;
        }

        match value {
            '.' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => break,
            '[' if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 => break,
            '(' => paren_depth += 1,
            ')' => {
                paren_depth = paren_depth
                    .checked_sub(1)
                    .ok_or_else(|| "unmatched `)` in JSONPath dot token".to_string())?;
            }
            '{' if paren_depth > 0 => brace_depth += 1,
            '}' if paren_depth > 0 => {
                brace_depth = brace_depth
                    .checked_sub(1)
                    .ok_or_else(|| "unmatched `}` in JSONPath dot token".to_string())?;
            }
            '[' if paren_depth > 0 => bracket_depth += 1,
            ']' if paren_depth > 0 => {
                bracket_depth = bracket_depth
                    .checked_sub(1)
                    .ok_or_else(|| "unmatched `]` in JSONPath dot token".to_string())?;
            }
            _ => {}
        }

        index += 1;
    }

    if quote.is_some() || paren_depth != 0 || brace_depth != 0 || bracket_depth != 0 {
        return Err("unterminated JSONPath dot token".to_string());
    }

    Ok(index)
}

fn parse_json_path_dot_token(token: &str) -> Result<JsonPathSegment, String> {
    match token {
        "length()" => Ok(JsonPathSegment::Function(JsonPathFunction::Length(None))),
        "first()" => Ok(JsonPathSegment::Function(JsonPathFunction::First)),
        "last()" => Ok(JsonPathSegment::Function(JsonPathFunction::Last)),
        "sum()" => Ok(JsonPathSegment::Function(JsonPathFunction::Sum(Vec::new()))),
        "min()" => Ok(JsonPathSegment::Function(JsonPathFunction::Min(Vec::new()))),
        "max()" => Ok(JsonPathSegment::Function(JsonPathFunction::Max(Vec::new()))),
        "avg()" => Ok(JsonPathSegment::Function(JsonPathFunction::Avg(Vec::new()))),
        "stddev()" => Ok(JsonPathSegment::Function(JsonPathFunction::StdDev(
            Vec::new(),
        ))),
        "keys()" => Ok(JsonPathSegment::Function(JsonPathFunction::Keys)),
        "values()" => Ok(JsonPathSegment::Function(JsonPathFunction::Values)),
        "size()" => Ok(JsonPathSegment::Function(JsonPathFunction::Length(None))),
        value if value.starts_with("length(") && value.ends_with(')') => {
            let arguments = value["length(".len()..value.len() - 1].trim();
            let argument = parse_json_path_single_path_argument(arguments).map_err(|err| {
                format!("invalid JSONPath length() arguments `{arguments}`: {err}")
            })?;
            Ok(JsonPathSegment::Function(JsonPathFunction::Length(Some(
                argument,
            ))))
        }
        value if value.starts_with("size(") && value.ends_with(')') => {
            let arguments = value["size(".len()..value.len() - 1].trim();
            let argument = parse_json_path_single_path_argument(arguments)
                .map_err(|err| format!("invalid JSONPath size() arguments `{arguments}`: {err}"))?;
            Ok(JsonPathSegment::Function(JsonPathFunction::Length(Some(
                argument,
            ))))
        }
        value if value.starts_with("concat(") && value.ends_with(')') => {
            let arguments = value["concat(".len()..value.len() - 1].trim();
            let arguments = parse_json_path_value_arguments(arguments).map_err(|err| {
                format!("invalid JSONPath concat() arguments `{arguments}`: {err}")
            })?;
            Ok(JsonPathSegment::Function(JsonPathFunction::Concat(
                arguments,
            )))
        }
        value if value.starts_with("append(") && value.ends_with(')') => {
            let arguments = value["append(".len()..value.len() - 1].trim();
            let arguments = parse_json_path_value_arguments(arguments).map_err(|err| {
                format!("invalid JSONPath append() arguments `{arguments}`: {err}")
            })?;
            Ok(JsonPathSegment::Function(JsonPathFunction::Append(
                arguments,
            )))
        }
        value if value.starts_with("sum(") && value.ends_with(')') => {
            let arguments = value["sum(".len()..value.len() - 1].trim();
            let arguments = parse_json_path_value_arguments(arguments)
                .map_err(|err| format!("invalid JSONPath sum() arguments `{arguments}`: {err}"))?;
            Ok(JsonPathSegment::Function(JsonPathFunction::Sum(arguments)))
        }
        value if value.starts_with("min(") && value.ends_with(')') => {
            let arguments = value["min(".len()..value.len() - 1].trim();
            let arguments = parse_json_path_value_arguments(arguments)
                .map_err(|err| format!("invalid JSONPath min() arguments `{arguments}`: {err}"))?;
            Ok(JsonPathSegment::Function(JsonPathFunction::Min(arguments)))
        }
        value if value.starts_with("max(") && value.ends_with(')') => {
            let arguments = value["max(".len()..value.len() - 1].trim();
            let arguments = parse_json_path_value_arguments(arguments)
                .map_err(|err| format!("invalid JSONPath max() arguments `{arguments}`: {err}"))?;
            Ok(JsonPathSegment::Function(JsonPathFunction::Max(arguments)))
        }
        value if value.starts_with("avg(") && value.ends_with(')') => {
            let arguments = value["avg(".len()..value.len() - 1].trim();
            let arguments = parse_json_path_value_arguments(arguments)
                .map_err(|err| format!("invalid JSONPath avg() arguments `{arguments}`: {err}"))?;
            Ok(JsonPathSegment::Function(JsonPathFunction::Avg(arguments)))
        }
        value if value.starts_with("stddev(") && value.ends_with(')') => {
            let arguments = value["stddev(".len()..value.len() - 1].trim();
            let arguments = parse_json_path_value_arguments(arguments).map_err(|err| {
                format!("invalid JSONPath stddev() arguments `{arguments}`: {err}")
            })?;
            Ok(JsonPathSegment::Function(JsonPathFunction::StdDev(
                arguments,
            )))
        }
        value if value.starts_with("index(") && value.ends_with(')') => {
            let arguments = value["index(".len()..value.len() - 1].trim();
            let argument = parse_json_path_single_value_argument(arguments)
                .map_err(|err| format!("invalid JSONPath index() argument `{arguments}`: {err}"))?;
            Ok(JsonPathSegment::Function(JsonPathFunction::Index(argument)))
        }
        value if value.contains('(') && value.ends_with(')') => {
            Err(format!("unsupported JSONPath function `{value}`"))
        }
        value => Ok(JsonPathSegment::Field(value.to_string())),
    }
}

fn parse_recursive_descent(chars: &[char], index: &mut usize) -> Result<JsonPathSegment, String> {
    *index += 1;
    if *index >= chars.len() {
        return Err("segment expected after `..`".to_string());
    }

    let inner = if chars[*index] == '*' {
        *index += 1;
        JsonPathSegment::Wildcard
    } else if chars[*index] == '[' {
        *index += 1;
        let (segment, next_index) = parse_json_path_bracket(chars, *index)?;
        *index = next_index;
        segment
    } else {
        let start = *index;
        while *index < chars.len() && chars[*index] != '.' && chars[*index] != '[' {
            *index += 1;
        }
        if start == *index {
            return Err("field name expected after `..`".to_string());
        }
        JsonPathSegment::Field(chars[start..*index].iter().collect())
    };

    Ok(JsonPathSegment::RecursiveDescent(Box::new(inner)))
}

fn terminal_json_path_function(
    segments: &[JsonPathSegment],
) -> Result<Option<(&[JsonPathSegment], JsonPathFunction)>, String> {
    for (index, segment) in segments.iter().enumerate() {
        if let JsonPathSegment::Function(function) = segment {
            if index + 1 != segments.len() {
                return Err("JSONPath functions must be terminal path segments".to_string());
            }
            return Ok(Some((&segments[..index], function.clone())));
        }
    }

    Ok(None)
}

fn parse_json_path_bracket(
    chars: &[char],
    index: usize,
) -> Result<(JsonPathSegment, usize), String> {
    if index >= chars.len() {
        return Err("unterminated `[` segment".to_string());
    }

    let start = index;
    let index = scan_json_path_bracket_token_end(chars, index)?;

    let token = chars[start..index]
        .iter()
        .collect::<String>()
        .trim()
        .to_string();
    let segment = parse_json_path_bracket_token(&token)?;

    Ok((segment, index + 1))
}

fn parse_json_path_bracket_token(token: &str) -> Result<JsonPathSegment, String> {
    let token = token.trim();
    let union_items = split_json_path_union_items(token)?;
    if union_items.len() > 1 {
        let segments = union_items
            .into_iter()
            .map(parse_json_path_bracket_token)
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(JsonPathSegment::Union(segments));
    }

    let segment = if token == "*" {
        JsonPathSegment::Wildcard
    } else if token.starts_with('\'') || token.starts_with('"') {
        JsonPathSegment::Field(parse_json_path_quoted_field_token(token)?)
    } else if token.starts_with("?(") {
        parse_json_path_filter(&token)?
    } else if token.contains(':') {
        parse_json_path_slice(&token)?
    } else if let Ok(signed) = token.parse::<isize>() {
        if signed >= 0 {
            JsonPathSegment::Index(signed as usize)
        } else {
            JsonPathSegment::IndexFromEnd((-signed) as usize)
        }
    } else {
        return Err(format!("unsupported bracket segment `{token}`"));
    };

    Ok(segment)
}

fn parse_json_path_quoted_field_token(token: &str) -> Result<String, String> {
    let chars = token.chars().collect::<Vec<_>>();
    let Some(&quote @ ('\'' | '"')) = chars.first() else {
        return Err(format!(
            "quoted field segment must start with a quote in `{token}`"
        ));
    };

    let mut index = 1;
    let mut field = String::new();

    while index < chars.len() {
        match chars[index] {
            '\\' if index + 1 < chars.len() => {
                index += 1;
                field.push(chars[index]);
                index += 1;
            }
            current if current == quote => {
                index += 1;
                if index == chars.len() {
                    return Ok(field);
                }
                return Err(format!(
                    "unexpected trailing quoted field text in `{token}`"
                ));
            }
            current => {
                field.push(current);
                index += 1;
            }
        }
    }

    Err(format!("unterminated quoted field segment in `{token}`"))
}

fn split_json_path_union_items(token: &str) -> Result<Vec<&str>, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut start = 0usize;
    let mut items = Vec::new();

    for (index, value) in token.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        match value {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ',' if paren_depth == 0 => {
                let item = token[start..index].trim();
                if item.is_empty() {
                    return Err(format!("empty union segment in `{token}`"));
                }
                items.push(item);
                start = index + 1;
            }
            _ => {}
        }
    }

    let item = token[start..].trim();
    if item.is_empty() {
        return Err(format!("empty union segment in `{token}`"));
    }
    items.push(item);

    Ok(items)
}

fn scan_json_path_bracket_token_end(chars: &[char], mut index: usize) -> Result<usize, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;

    while index < chars.len() {
        let current = chars[index];
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == active_quote {
                quote = None;
            }
            index += 1;
            continue;
        }

        if current == '\'' || current == '"' {
            quote = Some(current);
            index += 1;
            continue;
        }

        match current {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ']' if paren_depth == 0 => return Ok(index),
            _ => {}
        }
        index += 1;
    }

    Err("unterminated `[` segment".to_string())
}

fn parse_json_path_filter(token: &str) -> Result<JsonPathSegment, String> {
    let expression = token
        .strip_prefix("?(")
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| format!("invalid filter segment `{token}`"))?
        .trim();

    parse_filter_expression(expression).map(JsonPathSegment::Filter)
}

fn parse_filter_expression(expression: &str) -> Result<JsonPathFilter, String> {
    let mut expression = expression.trim();
    while let Some(inner) = strip_enclosing_filter_parentheses(expression) {
        expression = inner;
    }

    if let Some(inner) = expression.strip_prefix('!') {
        let inner = inner.trim();
        if inner.is_empty() {
            return Err(format!("empty filter condition in `{expression}`"));
        }

        return parse_filter_expression(inner).map(|filter| JsonPathFilter::Not(Box::new(filter)));
    }

    parse_filter_or_conditions(expression)
}

fn parse_filter_or_conditions(expression: &str) -> Result<JsonPathFilter, String> {
    let alternatives = split_filter_conditions(expression, &["||", " or "])?;
    if alternatives.len() > 1 {
        let filters = alternatives
            .into_iter()
            .map(parse_filter_expression)
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(JsonPathFilter::Any(filters));
    }

    parse_filter_and_conditions(expression)
}

fn parse_filter_and_conditions(expression: &str) -> Result<JsonPathFilter, String> {
    let conditions = split_filter_conditions(expression, &["&&", " and "])?;
    if conditions.len() > 1 {
        let filters = conditions
            .into_iter()
            .map(parse_filter_expression)
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(JsonPathFilter::All(filters));
    }

    parse_single_json_path_filter(expression)
}

fn parse_single_json_path_filter(expression: &str) -> Result<JsonPathFilter, String> {
    let Some((operator_index, operator_len, op)) = find_filter_operator(expression) else {
        if let Some(inner) = filter_not_function_inner(expression) {
            let path = parse_filter_path(inner)?;
            return Ok(JsonPathFilter::Not(Box::new(JsonPathFilter::Exists(path))));
        }

        let path = parse_filter_path(expression)?;
        return Ok(JsonPathFilter::Exists(path));
    };
    let lhs = expression[..operator_index].trim();
    let rhs = expression[operator_index + operator_len..].trim();

    if op == JsonPathFilterOp::Empty {
        if !rhs.is_empty() {
            return Err(format!(
                "filter empty operator does not accept RHS in `{expression}`"
            ));
        }
        return parse_filter_path(lhs).map(JsonPathFilter::Empty);
    }
    let left = parse_filter_value_ref(lhs)?;
    let right = parse_filter_operand(rhs)?;

    Ok(JsonPathFilter::Compare(JsonPathFilterComparison {
        left,
        op,
        right,
    }))
}

fn strip_enclosing_filter_parentheses(expression: &str) -> Option<&str> {
    let expression = expression.trim();
    if !expression.starts_with('(') || !expression.ends_with(')') {
        return None;
    }

    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

    for (index, value) in expression.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' if bracket_depth == 0 => paren_depth += 1,
            ')' if bracket_depth == 0 => {
                paren_depth = paren_depth.saturating_sub(1);
                if paren_depth == 0 {
                    if index + value.len_utf8() == expression.len() {
                        return Some(expression[1..index].trim());
                    }
                    return None;
                }
            }
            _ => {}
        }
    }

    None
}

fn filter_not_function_inner(expression: &str) -> Option<&str> {
    let expression = expression.trim();
    expression
        .strip_prefix("not(")
        .and_then(|value| value.strip_suffix(')'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn split_filter_conditions<'a>(
    expression: &'a str,
    separators: &[&str],
) -> Result<Vec<&'a str>, String> {
    if separators.is_empty() || separators.iter().any(|separator| separator.is_empty()) {
        return Err("filter condition operator must not be empty".to_string());
    }

    let mut quote = None;
    let mut escaped = false;
    let mut in_slash_regex = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut start = 0usize;
    let mut conditions = Vec::new();

    for (index, value) in expression.char_indices() {
        if in_slash_regex {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == '/' {
                in_slash_regex = false;
            }
            continue;
        }

        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        if value == '/' && expression[..index].trim_end().ends_with("=~") {
            in_slash_regex = true;
            continue;
        }

        if bracket_depth == 0 && paren_depth == 0 {
            if let Some(separator) = separators
                .iter()
                .find(|separator| expression[index..].starts_with(**separator))
            {
                let condition = expression[start..index].trim();
                if condition.is_empty() {
                    return Err(format!("empty filter condition in `{expression}`"));
                }
                conditions.push(condition);
                start = index + separator.len();
                continue;
            }
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }
    }

    let condition = expression[start..].trim();
    if condition.is_empty() {
        return Err(format!("empty filter condition in `{expression}`"));
    }
    conditions.push(condition);

    Ok(conditions)
}

fn find_filter_operator(expression: &str) -> Option<(usize, usize, JsonPathFilterOp)> {
    let mut quote = None;
    let mut escaped = false;

    for (index, value) in expression.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        if filter_word_operator_at(expression, index, "subsetof") {
            return Some((index, "subsetof".len(), JsonPathFilterOp::SubsetOf));
        }
        if filter_word_operator_at(expression, index, "size") {
            return Some((index, "size".len(), JsonPathFilterOp::Size));
        }
        if filter_word_operator_at(expression, index, "empty") {
            return Some((index, "empty".len(), JsonPathFilterOp::Empty));
        }
        if filter_word_operator_at(expression, index, "noneof") {
            return Some((index, "noneof".len(), JsonPathFilterOp::NoneOf));
        }
        if filter_word_operator_at(expression, index, "anyof") {
            return Some((index, "anyof".len(), JsonPathFilterOp::AnyOf));
        }
        if filter_word_operator_at(expression, index, "nin") {
            return Some((index, "nin".len(), JsonPathFilterOp::NotIn));
        }
        if filter_word_operator_at(expression, index, "in") {
            return Some((index, "in".len(), JsonPathFilterOp::In));
        }

        let remaining = &expression[index..];
        if remaining.starts_with("==") {
            return Some((index, 2, JsonPathFilterOp::Equal));
        }
        if remaining.starts_with("!=") {
            return Some((index, 2, JsonPathFilterOp::NotEqual));
        }
        if remaining.starts_with("=~") {
            return Some((index, 2, JsonPathFilterOp::RegexMatch));
        }
        if remaining.starts_with(">=") {
            return Some((index, 2, JsonPathFilterOp::GreaterThanOrEqual));
        }
        if remaining.starts_with("<=") {
            return Some((index, 2, JsonPathFilterOp::LessThanOrEqual));
        }
        if remaining.starts_with('>') {
            return Some((index, 1, JsonPathFilterOp::GreaterThan));
        }
        if remaining.starts_with('<') {
            return Some((index, 1, JsonPathFilterOp::LessThan));
        }
    }

    None
}

fn filter_word_operator_at(expression: &str, index: usize, operator: &str) -> bool {
    if !expression[index..].starts_with(operator) {
        return false;
    }

    let before = expression[..index].chars().next_back();
    let after = expression[index + operator.len()..].chars().next();

    before.is_some_and(char::is_whitespace) && !after.is_some_and(is_filter_word_char)
}

fn is_filter_word_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || value == '_' || value == '$'
}

fn parse_filter_value_ref(lhs: &str) -> Result<JsonPathFilterValueRef, String> {
    if lhs.trim() == "@" {
        return Ok(JsonPathFilterValueRef::Current);
    }

    parse_filter_path(lhs).map(JsonPathFilterValueRef::Path)
}

fn parse_filter_path(lhs: &str) -> Result<Vec<String>, String> {
    let chars = lhs.trim().chars().collect::<Vec<_>>();
    if chars.first() != Some(&'@') {
        return Err(format!("filter field must start with `@` in `{lhs}`"));
    }

    let mut fields = Vec::new();
    let mut index = 1;

    while index < chars.len() {
        match chars[index] {
            '.' => {
                index += 1;
                let start = index;
                while index < chars.len() && chars[index] != '.' && chars[index] != '[' {
                    index += 1;
                }
                if start == index {
                    return Err(format!("filter field path is empty in `{lhs}`"));
                }
                fields.push(chars[start..index].iter().collect());
            }
            '[' => {
                fields.push(parse_filter_quoted_field(lhs, &chars, &mut index)?);
            }
            current => {
                return Err(format!(
                    "expected `.` or `[` in filter field path `{lhs}`, found `{current}`"
                ));
            }
        }
    }

    if fields.is_empty() || fields.iter().any(String::is_empty) {
        return Err(format!("filter field path is empty in `{lhs}`"));
    }

    Ok(fields)
}

fn parse_filter_quoted_field(
    lhs: &str,
    chars: &[char],
    index: &mut usize,
) -> Result<String, String> {
    *index += 1;
    if *index >= chars.len() {
        return Err(format!("unterminated filter field bracket in `{lhs}`"));
    }

    let quote = chars[*index];
    if quote != '\'' && quote != '"' {
        return Err(format!(
            "filter field bracket must contain a quoted key in `{lhs}`"
        ));
    }
    *index += 1;

    let mut field = String::new();
    while *index < chars.len() {
        match chars[*index] {
            '\\' if *index + 1 < chars.len() => {
                *index += 1;
                field.push(chars[*index]);
                *index += 1;
            }
            current if current == quote => {
                *index += 1;
                if chars.get(*index) != Some(&']') {
                    return Err(format!(
                        "quoted filter field must close with `]` in `{lhs}`"
                    ));
                }
                *index += 1;
                return Ok(field);
            }
            current => {
                field.push(current);
                *index += 1;
            }
        }
    }

    Err(format!("unterminated quoted filter field in `{lhs}`"))
}

fn parse_filter_operand(rhs: &str) -> Result<JsonPathFilterOperand, String> {
    if rhs.trim().starts_with('@') {
        return parse_filter_value_ref(rhs).map(JsonPathFilterOperand::Path);
    }

    parse_filter_literal(rhs).map(JsonPathFilterOperand::Literal)
}

fn parse_filter_literal(rhs: &str) -> Result<JsonPathFilterLiteral, String> {
    if rhs.starts_with('[') {
        return parse_filter_array_literal(rhs).map(JsonPathFilterLiteral::Array);
    }

    if rhs.starts_with('/') {
        return parse_filter_regex_literal(rhs).map(JsonPathFilterLiteral::Regex);
    }

    if rhs.starts_with('\'') || rhs.starts_with('"') {
        return parse_filter_string_literal(rhs).map(JsonPathFilterLiteral::String);
    }

    match serde_json::from_str::<JsonValue>(rhs) {
        Ok(JsonValue::Number(value)) => Ok(JsonPathFilterLiteral::Number(value.to_string())),
        Ok(JsonValue::Bool(value)) => Ok(JsonPathFilterLiteral::Bool(value)),
        Ok(JsonValue::Null) => Ok(JsonPathFilterLiteral::Null),
        Ok(_) => Err(format!("unsupported filter comparison literal `{rhs}`")),
        Err(_) => Err(format!("unsupported filter comparison literal `{rhs}`")),
    }
}

fn parse_filter_array_literal(rhs: &str) -> Result<Vec<JsonPathFilterLiteral>, String> {
    let rhs = rhs.trim();
    if !rhs.starts_with('[') || !rhs.ends_with(']') {
        return Err(format!(
            "filter array literal must be enclosed by `[]` in `{rhs}`"
        ));
    }

    let inner = rhs[1..rhs.len() - 1].trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }

    let mut values = Vec::new();
    for item in split_filter_array_items(inner)? {
        let literal = parse_filter_literal(item.trim())?;
        if matches!(literal, JsonPathFilterLiteral::Array(_)) {
            return Err(format!("nested filter arrays are unsupported in `{rhs}`"));
        }
        values.push(literal);
    }

    Ok(values)
}

fn split_filter_array_items(inner: &str) -> Result<Vec<&str>, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut start = 0usize;
    let mut items = Vec::new();

    for (index, value) in inner.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        if value == ',' {
            let item = inner[start..index].trim();
            if item.is_empty() {
                return Err(format!("empty filter array item in `[{inner}]`"));
            }
            items.push(item);
            start = index + value.len_utf8();
        }
    }

    if quote.is_some() {
        return Err(format!(
            "unterminated quoted filter array item in `[{inner}]`"
        ));
    }

    let item = inner[start..].trim();
    if item.is_empty() {
        return Err(format!("empty filter array item in `[{inner}]`"));
    }
    items.push(item);

    Ok(items)
}

fn parse_filter_regex_literal(rhs: &str) -> Result<JsonPathFilterRegex, String> {
    let mut chars = rhs.char_indices();
    let Some((_, '/')) = chars.next() else {
        return Err(format!(
            "filter regex literal must start with `/` in `{rhs}`"
        ));
    };

    let mut pattern = String::new();
    let mut escaped = false;

    for (index, value) in chars {
        if escaped {
            if value == '/' {
                pattern.push('/');
            } else {
                pattern.push('\\');
                pattern.push(value);
            }
            escaped = false;
            continue;
        }

        match value {
            '\\' => escaped = true,
            '/' => {
                let flags = rhs[index + value.len_utf8()..].trim();
                let regex = JsonPathFilterRegex {
                    pattern,
                    flags: flags.to_string(),
                };
                build_filter_regex(&regex)
                    .map_err(|err| format!("invalid filter regex literal `{rhs}`: {err}"))?;
                return Ok(regex);
            }
            current => pattern.push(current),
        }
    }

    Err(format!("unterminated filter regex literal `{rhs}`"))
}

fn build_filter_regex(regex: &JsonPathFilterRegex) -> Result<Regex, String> {
    let mut builder = RegexBuilder::new(&regex.pattern);

    for flag in regex.flags.chars() {
        match flag {
            'i' => {
                builder.case_insensitive(true);
            }
            'm' => {
                builder.multi_line(true);
            }
            's' => {
                builder.dot_matches_new_line(true);
            }
            'x' => {
                builder.ignore_whitespace(true);
            }
            'U' => {
                builder.swap_greed(true);
            }
            'u' => {}
            _ => {
                return Err(format!("unsupported filter regex flag `{flag}`"));
            }
        }
    }

    builder.build().map_err(|err| err.to_string())
}

fn parse_filter_string_literal(rhs: &str) -> Result<String, String> {
    let mut chars = rhs.char_indices();
    let Some((_, quote @ ('\'' | '"'))) = chars.next() else {
        return Err(format!(
            "filter comparison value must be a string literal in `{rhs}`"
        ));
    };

    let mut output = String::new();
    let mut escaped = false;
    for (index, value) in chars {
        if escaped {
            output.push(value);
            escaped = false;
            continue;
        }

        if value == '\\' {
            escaped = true;
            continue;
        }

        if value == quote {
            if !rhs[index + value.len_utf8()..].trim().is_empty() {
                return Err(format!("unexpected trailing filter text in `{rhs}`"));
            }
            return Ok(output);
        }

        output.push(value);
    }

    Err(format!("unterminated filter string literal in `{rhs}`"))
}

fn parse_json_path_slice(token: &str) -> Result<JsonPathSegment, String> {
    let parts: Vec<&str> = token.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(format!("invalid slice segment `{token}`"));
    }

    let parse_opt = |raw: &str| -> Result<Option<isize>, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            trimmed
                .parse::<isize>()
                .map(Some)
                .map_err(|_| format!("invalid slice index `{trimmed}` in `{token}`"))
        }
    };

    let start = parse_opt(parts[0])?;
    let end = parse_opt(parts[1])?;
    let step = if parts.len() == 3 {
        parse_opt(parts[2])?
    } else {
        None
    };

    if matches!(step, Some(0)) {
        return Err(format!("slice step cannot be zero in `{token}`"));
    }

    Ok(JsonPathSegment::Slice(JsonPathSlice { start, end, step }))
}

fn evaluate_json_path<'a>(root: &'a JsonValue, segments: &[JsonPathSegment]) -> Vec<&'a JsonValue> {
    let mut current = vec![root];

    for segment in segments {
        let mut next = Vec::new();
        for value in &current {
            apply_json_path_segment(segment, value, &mut next);
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }

    current
}

fn apply_json_path_function(
    root: &JsonValue,
    function: JsonPathFunction,
    input: Vec<&JsonValue>,
) -> Vec<JsonValue> {
    match function {
        JsonPathFunction::Length(argument) => {
            json_path_length_function_values(input, argument.as_deref())
        }
        JsonPathFunction::First => input
            .into_iter()
            .filter_map(json_path_first_value)
            .collect(),
        JsonPathFunction::Last => input.into_iter().filter_map(json_path_last_value).collect(),
        JsonPathFunction::Index(argument) => {
            json_path_index_function_values(root, input, &argument)
        }
        JsonPathFunction::Sum(arguments) => json_path_sum_function_values(root, input, &arguments),
        JsonPathFunction::Min(arguments) => json_path_min_function_values(root, input, &arguments),
        JsonPathFunction::Max(arguments) => json_path_max_function_values(root, input, &arguments),
        JsonPathFunction::Avg(arguments) => json_path_avg_function_values(root, input, &arguments),
        JsonPathFunction::StdDev(arguments) => {
            json_path_stddev_function_values(root, input, &arguments)
        }
        JsonPathFunction::Keys => json_path_keys_function_values(input),
        JsonPathFunction::Values => json_path_values_function_values(input),
        JsonPathFunction::Concat(arguments) => {
            json_path_concat_function_values(root, input, &arguments)
        }
        JsonPathFunction::Append(arguments) => {
            json_path_append_function_values(root, input, &arguments)
        }
    }
}

fn json_path_length_value(value: &JsonValue) -> Option<JsonValue> {
    let length = match value {
        JsonValue::Array(values) => values.len(),
        JsonValue::Object(object) => object.len(),
        JsonValue::String(text) => text.chars().count(),
        _ => return None,
    };

    let length = serde_json::Number::from(length as u64);
    Some(JsonValue::Number(length))
}

fn json_path_length_function_values(
    input: Vec<&JsonValue>,
    argument: Option<&[JsonPathSegment]>,
) -> Vec<JsonValue> {
    match argument {
        Some(argument) => input
            .into_iter()
            .filter_map(|value| json_path_length_argument_value(value, argument))
            .collect(),
        None => input
            .into_iter()
            .filter_map(json_path_length_value)
            .collect(),
    }
}

fn json_path_length_argument_value(
    value: &JsonValue,
    argument: &[JsonPathSegment],
) -> Option<JsonValue> {
    let values = evaluate_json_path_with_terminal_function(value, argument);
    if values.len() == 1 {
        return json_path_length_value(values.first()?);
    }

    let length = serde_json::Number::from(values.len() as u64);
    Some(JsonValue::Number(length))
}

fn json_path_first_value(value: &JsonValue) -> Option<JsonValue> {
    let JsonValue::Array(values) = value else {
        return None;
    };

    values.first().cloned()
}

fn json_path_last_value(value: &JsonValue) -> Option<JsonValue> {
    let JsonValue::Array(values) = value else {
        return None;
    };

    values.last().cloned()
}

fn json_path_index_function_values(
    root: &JsonValue,
    input: Vec<&JsonValue>,
    argument: &JsonPathFunctionArgument,
) -> Vec<JsonValue> {
    let Some(index) = json_path_index_function_argument(root, argument) else {
        return Vec::new();
    };

    input
        .into_iter()
        .filter_map(|value| json_path_index_function_value(value, index))
        .collect()
}

fn json_path_index_function_value(value: &JsonValue, index: isize) -> Option<JsonValue> {
    let JsonValue::Array(values) = value else {
        return None;
    };

    let index = resolve_json_path_function_index(index, values.len())?;
    values.get(index).cloned()
}

fn json_path_index_function_argument(
    root: &JsonValue,
    argument: &JsonPathFunctionArgument,
) -> Option<isize> {
    let value = json_path_function_argument_value(root, argument);
    json_path_function_argument_first_number(&value).and_then(json_number_to_isize)
}

fn json_path_function_argument_first_number(value: &JsonValue) -> Option<&serde_json::Number> {
    match value {
        JsonValue::Number(number) => Some(number),
        JsonValue::Array(values) => values.iter().find_map(|value| match value {
            JsonValue::Number(number) => Some(number),
            _ => None,
        }),
        _ => None,
    }
}

fn json_number_to_isize(number: &serde_json::Number) -> Option<isize> {
    number
        .as_i64()
        .and_then(|value| isize::try_from(value).ok())
        .or_else(|| {
            let value = number.as_f64()?;
            if value.fract() == 0.0 && value >= isize::MIN as f64 && value <= isize::MAX as f64 {
                Some(value as isize)
            } else {
                None
            }
        })
}

fn resolve_json_path_function_index(index: isize, len: usize) -> Option<usize> {
    if index >= 0 {
        return Some(index as usize).filter(|index| *index < len);
    }

    let offset = index.checked_neg()? as usize;
    len.checked_sub(offset)
}

fn json_path_sum_function_values(
    root: &JsonValue,
    input: Vec<&JsonValue>,
    arguments: &[JsonPathFunctionArgument],
) -> Vec<JsonValue> {
    let Some(values) = collect_json_path_numeric_function_values(root, input, arguments) else {
        return Vec::new();
    };

    let sum = values.into_iter().sum::<f64>();
    serde_json::Number::from_f64(sum)
        .map(JsonValue::Number)
        .into_iter()
        .collect()
}

fn json_path_min_function_values(
    root: &JsonValue,
    input: Vec<&JsonValue>,
    arguments: &[JsonPathFunctionArgument],
) -> Vec<JsonValue> {
    let Some(values) = collect_json_path_numeric_function_values(root, input, arguments) else {
        return Vec::new();
    };

    let Some(minimum) = values.into_iter().reduce(f64::min) else {
        return Vec::new();
    };

    serde_json::Number::from_f64(minimum)
        .map(JsonValue::Number)
        .into_iter()
        .collect()
}

fn json_path_max_function_values(
    root: &JsonValue,
    input: Vec<&JsonValue>,
    arguments: &[JsonPathFunctionArgument],
) -> Vec<JsonValue> {
    let Some(values) = collect_json_path_numeric_function_values(root, input, arguments) else {
        return Vec::new();
    };

    let Some(maximum) = values.into_iter().reduce(f64::max) else {
        return Vec::new();
    };

    serde_json::Number::from_f64(maximum)
        .map(JsonValue::Number)
        .into_iter()
        .collect()
}

fn json_path_avg_function_values(
    root: &JsonValue,
    input: Vec<&JsonValue>,
    arguments: &[JsonPathFunctionArgument],
) -> Vec<JsonValue> {
    let Some(values) = collect_json_path_numeric_function_values(root, input, arguments) else {
        return Vec::new();
    };
    if values.is_empty() {
        return Vec::new();
    }

    let average = values.iter().sum::<f64>() / values.len() as f64;
    serde_json::Number::from_f64(average)
        .map(JsonValue::Number)
        .into_iter()
        .collect()
}

fn json_path_stddev_function_values(
    root: &JsonValue,
    input: Vec<&JsonValue>,
    arguments: &[JsonPathFunctionArgument],
) -> Vec<JsonValue> {
    let Some(values) = collect_json_path_numeric_function_values(root, input, arguments) else {
        return Vec::new();
    };
    if values.is_empty() {
        return Vec::new();
    }

    let count = values.len() as f64;
    let sum = values.iter().sum::<f64>();
    let sum_squares = values.iter().map(|value| value * value).sum::<f64>();
    let standard_deviation = (sum_squares / count - sum * sum / count / count).sqrt();

    serde_json::Number::from_f64(standard_deviation)
        .map(JsonValue::Number)
        .into_iter()
        .collect()
}

fn json_path_keys_function_values(input: Vec<&JsonValue>) -> Vec<JsonValue> {
    input
        .into_iter()
        .filter_map(JsonValue::as_object)
        .flat_map(|object| {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            keys.into_iter()
                .map(|key| JsonValue::String(key.to_string()))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn json_path_values_function_values(input: Vec<&JsonValue>) -> Vec<JsonValue> {
    input
        .into_iter()
        .flat_map(|value| match value {
            JsonValue::Array(values) => values.clone(),
            JsonValue::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort();
                keys.into_iter()
                    .filter_map(|key| object.get(key).cloned())
                    .collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect()
}

fn json_path_concat_function_values(
    root: &JsonValue,
    input: Vec<&JsonValue>,
    arguments: &[JsonPathFunctionArgument],
) -> Vec<JsonValue> {
    let mut output = String::new();
    for value in input {
        if let JsonValue::Array(values) = value {
            for item in values {
                if let JsonValue::String(text) = item {
                    output.push_str(text);
                }
            }
        }
    }
    for argument in arguments {
        let value = json_path_function_argument_value(root, argument);
        for text in json_path_function_argument_string_values(&value) {
            output.push_str(&text);
        }
    }

    vec![JsonValue::String(output)]
}

fn json_path_function_argument_string_values(value: &JsonValue) -> Vec<String> {
    match value {
        JsonValue::Array(values) => values
            .iter()
            .filter_map(json_path_function_argument_string_value)
            .collect(),
        value => json_path_function_argument_string_value(value)
            .into_iter()
            .collect(),
    }
}

fn json_path_function_argument_string_value(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Null => None,
        JsonValue::String(text) => Some(text.clone()),
        _ => Some(json_value_to_rule_text(value)),
    }
}

fn json_path_append_function_values(
    root: &JsonValue,
    input: Vec<&JsonValue>,
    arguments: &[JsonPathFunctionArgument],
) -> Vec<JsonValue> {
    let arguments = arguments
        .iter()
        .map(|argument| json_path_function_argument_value(root, argument))
        .collect::<Vec<_>>();

    input
        .into_iter()
        .filter_map(|value| {
            let JsonValue::Array(values) = value else {
                return None;
            };

            let mut values = values.clone();
            values.extend(arguments.iter().cloned());
            Some(JsonValue::Array(values))
        })
        .collect()
}

fn json_path_function_argument_value(
    root: &JsonValue,
    argument: &JsonPathFunctionArgument,
) -> JsonValue {
    match argument {
        JsonPathFunctionArgument::Value(value) => value.clone(),
        JsonPathFunctionArgument::Path(segments) => {
            let mut values = evaluate_json_path_with_terminal_function(root, segments);
            if values.len() == 1 {
                values.remove(0)
            } else {
                JsonValue::Array(values)
            }
        }
    }
}

fn evaluate_json_path_with_terminal_function(
    root: &JsonValue,
    segments: &[JsonPathSegment],
) -> Vec<JsonValue> {
    if json_path_segments_contain_function(segments) {
        evaluate_json_path_with_functions(root, segments)
    } else {
        evaluate_json_path(root, segments)
            .into_iter()
            .cloned()
            .collect()
    }
}

fn parse_json_path_value_arguments(
    arguments: &str,
) -> Result<Vec<JsonPathFunctionArgument>, String> {
    let arguments = arguments.trim();
    if arguments.is_empty() {
        return Ok(Vec::new());
    }

    split_json_path_function_arguments(arguments)?
        .into_iter()
        .map(|argument| parse_json_path_value_argument(argument.trim()))
        .collect()
}

fn parse_json_path_single_path_argument(arguments: &str) -> Result<Vec<JsonPathSegment>, String> {
    let arguments = arguments.trim();
    if arguments.is_empty() {
        return Err("expected one JSONPath argument".to_string());
    }

    let arguments = split_json_path_function_arguments(arguments)?;
    if arguments.len() != 1 {
        return Err("expected one JSONPath argument".to_string());
    }

    let argument = arguments[0].trim();
    if !argument.starts_with('$') {
        return Err("function argument must be a JSONPath".to_string());
    }

    let segments = parse_json_path(argument)?;
    terminal_json_path_function(&segments)?;
    Ok(segments)
}

fn parse_json_path_single_value_argument(
    arguments: &str,
) -> Result<JsonPathFunctionArgument, String> {
    let mut arguments = parse_json_path_value_arguments(arguments)?;
    if arguments.len() != 1 {
        return Err("expected one JSONPath function argument".to_string());
    }

    Ok(arguments.remove(0))
}

fn parse_json_path_value_argument(argument: &str) -> Result<JsonPathFunctionArgument, String> {
    if argument.starts_with('$') {
        let segments = parse_json_path(argument)?;
        terminal_json_path_function(&segments)?;
        return Ok(JsonPathFunctionArgument::Path(segments));
    }

    if argument.starts_with('\'') {
        return parse_filter_string_literal(argument)
            .map(JsonValue::String)
            .map(JsonPathFunctionArgument::Value);
    }

    serde_json::from_str::<JsonValue>(argument)
        .map(JsonPathFunctionArgument::Value)
        .map_err(|err| format!("invalid JSON argument `{argument}`: {err}"))
}

fn split_json_path_function_arguments(arguments: &str) -> Result<Vec<&str>, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut brace_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut start = 0usize;
    let mut items = Vec::new();

    for (index, value) in arguments.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        match value {
            '{' => {
                brace_depth += 1;
                continue;
            }
            '}' => {
                brace_depth = brace_depth.checked_sub(1).ok_or_else(|| {
                    format!("unmatched JSONPath function argument delimiter in `{arguments}`")
                })?;
                continue;
            }
            '[' => {
                bracket_depth += 1;
                continue;
            }
            ']' => {
                bracket_depth = bracket_depth.checked_sub(1).ok_or_else(|| {
                    format!("unmatched JSONPath function argument delimiter in `{arguments}`")
                })?;
                continue;
            }
            _ => {}
        }

        if value == ',' && brace_depth == 0 && bracket_depth == 0 {
            let item = arguments[start..index].trim();
            if item.is_empty() {
                return Err(format!("empty JSONPath function argument in `{arguments}`"));
            }
            items.push(item);
            start = index + value.len_utf8();
        }
    }

    if quote.is_some() {
        return Err(format!(
            "unterminated quoted JSONPath function argument in `{arguments}`"
        ));
    }
    if brace_depth != 0 || bracket_depth != 0 {
        return Err(format!(
            "unterminated JSONPath function argument delimiter in `{arguments}`"
        ));
    }

    let item = arguments[start..].trim();
    if item.is_empty() {
        return Err(format!("empty JSONPath function argument in `{arguments}`"));
    }
    items.push(item);

    Ok(items)
}

fn collect_json_path_numeric_function_input(input: Vec<&JsonValue>) -> Option<Vec<f64>> {
    if input.len() == 1 {
        if let Some(JsonValue::Array(values)) = input.first() {
            return values.iter().map(JsonValue::as_f64).collect();
        }
    }

    input.into_iter().map(JsonValue::as_f64).collect()
}

fn collect_json_path_numeric_function_values(
    root: &JsonValue,
    input: Vec<&JsonValue>,
    arguments: &[JsonPathFunctionArgument],
) -> Option<Vec<f64>> {
    let mut values = collect_json_path_numeric_function_input(input)?;
    for argument in arguments {
        let value = json_path_function_argument_value(root, argument);
        collect_json_path_numeric_argument_values(&value, &mut values);
    }
    Some(values)
}

fn collect_json_path_numeric_argument_values(value: &JsonValue, output: &mut Vec<f64>) {
    match value {
        JsonValue::Array(values) => {
            output.extend(values.iter().filter_map(JsonValue::as_f64));
        }
        value => {
            if let Some(value) = value.as_f64() {
                output.push(value);
            }
        }
    }
}

fn apply_json_path_segment<'a>(
    segment: &JsonPathSegment,
    value: &'a JsonValue,
    output: &mut Vec<&'a JsonValue>,
) {
    match segment {
        JsonPathSegment::Field(field) => {
            if let JsonValue::Object(object) = value {
                if let Some(child) = object.get(field) {
                    output.push(child);
                }
            }
        }
        JsonPathSegment::Index(index) => {
            if let JsonValue::Array(array) = value {
                if let Some(child) = array.get(*index) {
                    output.push(child);
                }
            }
        }
        JsonPathSegment::IndexFromEnd(offset) => {
            if let JsonValue::Array(array) = value {
                if let Some(child) = array.len().checked_sub(*offset).and_then(|i| array.get(i)) {
                    output.push(child);
                }
            }
        }
        JsonPathSegment::Wildcard => match value {
            JsonValue::Array(array) => output.extend(array.iter()),
            JsonValue::Object(object) => output.extend(object.values()),
            _ => {}
        },
        JsonPathSegment::RecursiveDescent(inner) => {
            for descendant in collect_json_descendants(value) {
                apply_json_path_segment(inner, descendant, output);
            }
        }
        JsonPathSegment::Union(segments) => {
            for segment in segments {
                apply_json_path_segment(segment, value, output);
            }
        }
        JsonPathSegment::Slice(slice) => {
            if let JsonValue::Array(array) = value {
                let len = array.len() as isize;
                let step = slice.step.unwrap_or(1);
                let (start, end) = resolve_slice_bounds(slice.start, slice.end, step, len);

                let mut index = start;
                if step > 0 {
                    while index < end {
                        if (0..len).contains(&index) {
                            if let Some(child) = array.get(index as usize) {
                                output.push(child);
                            }
                        }
                        index += step;
                    }
                } else {
                    while index > end {
                        if (0..len).contains(&index) {
                            if let Some(child) = array.get(index as usize) {
                                output.push(child);
                            }
                        }
                        index += step;
                    }
                }
            }
        }
        JsonPathSegment::Filter(filter) => {
            if let JsonValue::Array(array) = value {
                output.extend(
                    array
                        .iter()
                        .filter(|item| json_path_filter_matches(item, filter)),
                );
            }
        }
        JsonPathSegment::Function(_) => {}
    }
}

fn json_path_filter_matches(value: &JsonValue, filter: &JsonPathFilter) -> bool {
    match filter {
        JsonPathFilter::Exists(path) => resolve_filter_path(value, path).is_some(),
        JsonPathFilter::Empty(path) => {
            resolve_filter_path(value, path).is_some_and(filter_value_is_empty)
        }
        JsonPathFilter::All(filters) => filters
            .iter()
            .all(|filter| json_path_filter_matches(value, filter)),
        JsonPathFilter::Any(filters) => filters
            .iter()
            .any(|filter| json_path_filter_matches(value, filter)),
        JsonPathFilter::Not(filter) => !json_path_filter_matches(value, filter),
        JsonPathFilter::Compare(comparison) => {
            let Some(actual) = resolve_filter_value_ref(value, &comparison.left) else {
                return false;
            };

            match comparison.op {
                JsonPathFilterOp::Equal => filter_values_equal(value, actual, &comparison.right),
                JsonPathFilterOp::NotEqual => {
                    filter_values_comparable(value, actual, &comparison.right)
                        .is_some_and(|equal| !equal)
                }
                JsonPathFilterOp::RegexMatch => {
                    filter_value_matches_regex(actual, &comparison.right)
                }
                JsonPathFilterOp::In => filter_value_in_operand(value, actual, &comparison.right),
                JsonPathFilterOp::NotIn => {
                    !filter_value_in_operand(value, actual, &comparison.right)
                }
                JsonPathFilterOp::AnyOf => {
                    filter_array_has_any_operand(value, actual, &comparison.right)
                }
                JsonPathFilterOp::NoneOf => {
                    filter_array_has_no_operand(value, actual, &comparison.right)
                }
                JsonPathFilterOp::SubsetOf => {
                    filter_array_is_subset_operand(value, actual, &comparison.right)
                }
                JsonPathFilterOp::Size => {
                    filter_value_size_matches(value, actual, &comparison.right)
                }
                JsonPathFilterOp::Empty => false,
                JsonPathFilterOp::GreaterThan => {
                    compare_filter_numbers(value, actual, &comparison.right, |actual, expected| {
                        actual > expected
                    })
                }
                JsonPathFilterOp::GreaterThanOrEqual => {
                    compare_filter_numbers(value, actual, &comparison.right, |actual, expected| {
                        actual >= expected
                    })
                }
                JsonPathFilterOp::LessThan => {
                    compare_filter_numbers(value, actual, &comparison.right, |actual, expected| {
                        actual < expected
                    })
                }
                JsonPathFilterOp::LessThanOrEqual => {
                    compare_filter_numbers(value, actual, &comparison.right, |actual, expected| {
                        actual <= expected
                    })
                }
            }
        }
    }
}

fn filter_values_equal(
    context: &JsonValue,
    actual: &JsonValue,
    expected: &JsonPathFilterOperand,
) -> bool {
    filter_values_comparable(context, actual, expected).unwrap_or(false)
}

fn filter_value_matches_regex(actual: &JsonValue, expected: &JsonPathFilterOperand) -> bool {
    let JsonPathFilterOperand::Literal(JsonPathFilterLiteral::Regex(expected)) = expected else {
        return false;
    };
    let JsonValue::String(actual) = actual else {
        return false;
    };

    build_filter_regex(expected).is_ok_and(|regex| regex.is_match(actual))
}

fn filter_value_in_operand(
    context: &JsonValue,
    actual: &JsonValue,
    expected: &JsonPathFilterOperand,
) -> bool {
    match expected {
        JsonPathFilterOperand::Literal(JsonPathFilterLiteral::Array(expected)) => expected
            .iter()
            .any(|expected| filter_value_equals_literal(actual, expected).unwrap_or(false)),
        JsonPathFilterOperand::Path(value_ref) => {
            let Some(JsonValue::Array(expected)) = resolve_filter_value_ref(context, value_ref)
            else {
                return false;
            };
            expected
                .iter()
                .any(|expected| filter_json_values_equal(actual, expected).unwrap_or(false))
        }
        JsonPathFilterOperand::Literal(_) => false,
    }
}

fn filter_array_has_any_operand(
    context: &JsonValue,
    actual: &JsonValue,
    expected: &JsonPathFilterOperand,
) -> bool {
    let JsonValue::Array(actual) = actual else {
        return false;
    };

    match expected {
        JsonPathFilterOperand::Literal(JsonPathFilterLiteral::Array(expected)) => {
            actual.iter().any(|actual| {
                expected
                    .iter()
                    .any(|expected| filter_value_equals_literal(actual, expected).unwrap_or(false))
            })
        }
        JsonPathFilterOperand::Path(value_ref) => {
            let Some(JsonValue::Array(expected)) = resolve_filter_value_ref(context, value_ref)
            else {
                return false;
            };
            actual.iter().any(|actual| {
                expected
                    .iter()
                    .any(|expected| filter_json_values_equal(actual, expected).unwrap_or(false))
            })
        }
        JsonPathFilterOperand::Literal(_) => false,
    }
}

fn filter_array_has_no_operand(
    context: &JsonValue,
    actual: &JsonValue,
    expected: &JsonPathFilterOperand,
) -> bool {
    if !matches!(actual, JsonValue::Array(_)) {
        return false;
    }

    match expected {
        JsonPathFilterOperand::Literal(JsonPathFilterLiteral::Array(_))
        | JsonPathFilterOperand::Path(_) => {
            !filter_array_has_any_operand(context, actual, expected)
        }
        JsonPathFilterOperand::Literal(_) => false,
    }
}

fn filter_array_is_subset_operand(
    context: &JsonValue,
    actual: &JsonValue,
    expected: &JsonPathFilterOperand,
) -> bool {
    let JsonValue::Array(actual) = actual else {
        return false;
    };

    match expected {
        JsonPathFilterOperand::Literal(JsonPathFilterLiteral::Array(expected)) => {
            actual.iter().all(|actual| {
                expected
                    .iter()
                    .any(|expected| filter_value_equals_literal(actual, expected).unwrap_or(false))
            })
        }
        JsonPathFilterOperand::Path(value_ref) => {
            let Some(JsonValue::Array(expected)) = resolve_filter_value_ref(context, value_ref)
            else {
                return false;
            };
            actual.iter().all(|actual| {
                expected
                    .iter()
                    .any(|expected| filter_json_values_equal(actual, expected).unwrap_or(false))
            })
        }
        JsonPathFilterOperand::Literal(_) => false,
    }
}

fn filter_value_size_matches(
    context: &JsonValue,
    actual: &JsonValue,
    expected: &JsonPathFilterOperand,
) -> bool {
    let Some(expected) = resolve_filter_number_operand(context, expected) else {
        return false;
    };
    if expected.fract() != 0.0 || expected < 0.0 {
        return false;
    }

    let actual_size = match actual {
        JsonValue::Array(values) => values.len(),
        JsonValue::String(value) => value.chars().count(),
        _ => return false,
    };

    actual_size as f64 == expected
}

fn filter_value_is_empty(actual: &JsonValue) -> bool {
    match actual {
        JsonValue::Array(values) => values.is_empty(),
        JsonValue::String(value) => value.is_empty(),
        _ => false,
    }
}

fn filter_values_comparable(
    context: &JsonValue,
    actual: &JsonValue,
    expected: &JsonPathFilterOperand,
) -> Option<bool> {
    match expected {
        JsonPathFilterOperand::Literal(expected) => filter_value_equals_literal(actual, expected),
        JsonPathFilterOperand::Path(value_ref) => {
            let expected = resolve_filter_value_ref(context, value_ref)?;
            filter_json_values_equal(actual, expected)
        }
    }
}

fn filter_value_equals_literal(
    actual: &JsonValue,
    expected: &JsonPathFilterLiteral,
) -> Option<bool> {
    match (actual, expected) {
        (JsonValue::String(actual), JsonPathFilterLiteral::String(expected)) => {
            Some(actual == expected)
        }
        (JsonValue::Number(actual), JsonPathFilterLiteral::Number(expected)) => {
            let actual = actual.as_f64()?;
            let expected = expected.parse::<f64>().ok()?;
            Some(actual == expected)
        }
        (JsonValue::Bool(actual), JsonPathFilterLiteral::Bool(expected)) => {
            Some(actual == expected)
        }
        (JsonValue::Null, JsonPathFilterLiteral::Null) => Some(true),
        (_, JsonPathFilterLiteral::Regex(_)) => None,
        (_, JsonPathFilterLiteral::Array(_)) => None,
        _ => None,
    }
}

fn filter_json_values_equal(actual: &JsonValue, expected: &JsonValue) -> Option<bool> {
    match (actual, expected) {
        (JsonValue::String(actual), JsonValue::String(expected)) => Some(actual == expected),
        (JsonValue::Number(actual), JsonValue::Number(expected)) => {
            Some(actual.as_f64()? == expected.as_f64()?)
        }
        (JsonValue::Bool(actual), JsonValue::Bool(expected)) => Some(actual == expected),
        (JsonValue::Null, JsonValue::Null) => Some(true),
        _ => None,
    }
}

fn compare_filter_numbers(
    context: &JsonValue,
    actual: &JsonValue,
    expected: &JsonPathFilterOperand,
    compare: impl FnOnce(f64, f64) -> bool,
) -> bool {
    let Some(expected) = resolve_filter_number_operand(context, expected) else {
        return false;
    };

    let Some(actual) = actual.as_f64() else {
        return false;
    };

    compare(actual, expected)
}

fn resolve_filter_number_operand(
    context: &JsonValue,
    expected: &JsonPathFilterOperand,
) -> Option<f64> {
    match expected {
        JsonPathFilterOperand::Literal(JsonPathFilterLiteral::Number(expected)) => {
            expected.parse::<f64>().ok()
        }
        JsonPathFilterOperand::Path(value_ref) => {
            resolve_filter_value_ref(context, value_ref)?.as_f64()
        }
        _ => None,
    }
}

fn resolve_filter_value_ref<'a>(
    value: &'a JsonValue,
    value_ref: &JsonPathFilterValueRef,
) -> Option<&'a JsonValue> {
    match value_ref {
        JsonPathFilterValueRef::Current => Some(value),
        JsonPathFilterValueRef::Path(path) => resolve_filter_path(value, path),
    }
}

fn resolve_filter_path<'a>(value: &'a JsonValue, path: &[String]) -> Option<&'a JsonValue> {
    let mut current = value;
    for field in path {
        let JsonValue::Object(object) = current else {
            return None;
        };
        current = object.get(field)?;
    }

    Some(current)
}

/// Resolves optional, possibly-negative slice bounds against the array length.
/// For positive `step`, defaults are `start=0` / `end=len`. For negative
/// `step`, defaults are `start=len-1` / `end=-1` (so index 0 is still visited).
/// Out-of-range values are clamped to the valid window.
fn resolve_slice_bounds(
    start: Option<isize>,
    end: Option<isize>,
    step: isize,
    len: isize,
) -> (isize, isize) {
    if step > 0 {
        let start = start.unwrap_or(0);
        let start = if start < 0 {
            (len + start).max(0)
        } else {
            start.min(len)
        };
        let end = end.unwrap_or(len);
        let end = if end < 0 {
            (len + end).max(0)
        } else {
            end.min(len)
        };
        (start, end)
    } else {
        let start = start.unwrap_or(len - 1);
        let start = if start < 0 {
            (len + start).max(-1)
        } else {
            start.min(len - 1)
        };
        let end = end.unwrap_or(-1);
        let end = if end < 0 {
            (len + end).max(-1)
        } else {
            end.min(len - 1)
        };
        (start, end)
    }
}

fn collect_json_descendants<'a>(value: &'a JsonValue) -> Vec<&'a JsonValue> {
    let mut result = vec![value];
    match value {
        JsonValue::Array(array) => {
            for item in array {
                result.extend(collect_json_descendants(item));
            }
        }
        JsonValue::Object(object) => {
            for child in object.values() {
                result.extend(collect_json_descendants(child));
            }
        }
        _ => {}
    }
    result
}

fn json_value_to_rule_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(value) => value.clone(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null => "null".to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| String::new())
        }
    }
}

fn parse_legado_css_rule(rule: &str) -> Result<LegadoCssRule, String> {
    if rule.trim().is_empty() {
        return Ok(LegadoCssRule { steps: Vec::new() });
    }

    let parts = split_legado_css_pipeline(rule)?;
    let mut steps = Vec::with_capacity(parts.len());

    for (index, part) in parts.iter().enumerate() {
        let part_steps = parse_legado_css_step(part)?;
        let has_extract = part_steps
            .iter()
            .any(|step| matches!(step, LegadoCssStep::Extract { .. }));
        // 抽取步骤只能是整条管道的末尾,不能被后续 select 跟随。
        if has_extract && index + 1 != parts.len() {
            return Err("extraction step must be the final pipeline segment".to_string());
        }
        steps.extend(part_steps);
    }

    Ok(LegadoCssRule { steps })
}

fn split_legado_css_pipeline(rule: &str) -> Result<Vec<&str>, String> {
    let mut parts = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut start = 0usize;
    let mut index = 0usize;

    while index < rule.len() {
        let value = rule[index..]
            .chars()
            .next()
            .ok_or_else(|| "invalid Legado CSS rule".to_string())?;

        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            index += value.len_utf8();
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            index += value.len_utf8();
            continue;
        }

        if bracket_depth == 0 && paren_depth == 0 {
            if value == ';' {
                parts.push(&rule[start..index]);
                index += value.len_utf8();
                start = index;
                continue;
            }

            if rule[index..].starts_with("&&") {
                parts.push(&rule[start..index]);
                index += "&&".len();
                start = index;
                continue;
            }
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => {
                if bracket_depth == 0 {
                    return Err("unmatched `]` in selector".to_string());
                }
                bracket_depth -= 1;
            }
            '(' => paren_depth += 1,
            ')' => {
                if paren_depth == 0 {
                    return Err("unmatched `)` in selector".to_string());
                }
                paren_depth -= 1;
            }
            _ => {}
        }

        index += value.len_utf8();
    }

    if quote.is_some() {
        return Err("unterminated quoted selector segment".to_string());
    }
    if bracket_depth != 0 {
        return Err("unterminated attribute selector".to_string());
    }
    if paren_depth != 0 {
        return Err("unterminated selector function".to_string());
    }

    parts.push(&rule[start..]);
    Ok(parts)
}

fn parse_legado_css_step(part: &str) -> Result<Vec<LegadoCssStep>, String> {
    let part = part.trim();
    if part.is_empty() {
        return Err("empty pipeline segment".to_string());
    }

    // Legado AnalyzeByJSoup.kt getResultList (line 200-224):用 `@` 切割规则,
    // 前 n-1 段是 select(逐级缩小元素集),最后一段是 getResultLast 抽取。
    // 例如 `tag.h3@tag.a@text` -> [Select("tag.h3"), Extract { Some("tag.a"), Text }]。
    let segments = split_legado_css_at_pipeline(part)?;

    if segments.len() == 1 {
        // 没有 `@`:整段是 select,抽取由管道末尾的默认 text 兜底。
        return Ok(vec![LegadoCssStep::Select(segments[0].trim().to_string())]);
    }

    // 2+ 段:最后一段是抽取,倒数第二段是抽取步骤的 selector,前面所有段是独立 Select。
    let last_idx = segments.len() - 1;
    let selector_idx = last_idx - 1;
    let mut steps = Vec::with_capacity(segments.len());

    for seg in &segments[..selector_idx] {
        let seg = seg.trim();
        if seg.is_empty() {
            return Err("empty selector in pipeline segment".to_string());
        }
        steps.push(LegadoCssStep::Select(seg.to_string()));
    }

    let selector_str = segments[selector_idx].trim();
    let selector = if selector_str.is_empty() {
        None
    } else {
        Some(selector_str.to_string())
    };

    let extraction_str = segments[last_idx].trim();
    if extraction_str.is_empty() {
        return Err("missing extraction after `@`".to_string());
    }
    let extraction = parse_legado_css_extraction(extraction_str);

    steps.push(LegadoCssStep::Extract { selector, extraction });

    Ok(steps)
}

fn parse_legado_css_extraction(extraction: &str) -> LegadoCssExtraction {
    if extraction.eq_ignore_ascii_case("text") {
        LegadoCssExtraction::Text
    } else if extraction.eq_ignore_ascii_case("html") {
        LegadoCssExtraction::Html
    } else {
        LegadoCssExtraction::Attr(extraction.to_string())
    }
}

/// 按 top-level `@` 切分 Legado CSS 管道段,尊重引号/方括号/圆括号嵌套。
///
/// 对应 Legado `AnalyzeByJSoup.kt` `RuleAnalyzer.splitRule("@")` 的语义。
/// 例如 `tag.h3@tag.a@text` -> ["tag.h3", "tag.a", "text"],
/// `a.book@text` -> ["a.book", "text"],`@href` -> ["", "href"]。
fn split_legado_css_at_pipeline(part: &str) -> Result<Vec<&str>, String> {
    let mut segments = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut start = 0usize;

    for (index, value) in part.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        if value == '@' && bracket_depth == 0 && paren_depth == 0 {
            segments.push(&part[start..index]);
            start = index + '@'.len_utf8();
            continue;
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => {
                if bracket_depth == 0 {
                    return Err("unmatched `]` in selector".to_string());
                }
                bracket_depth -= 1;
            }
            '(' => paren_depth += 1,
            ')' => {
                if paren_depth == 0 {
                    return Err("unmatched `)` in selector".to_string());
                }
                paren_depth -= 1;
            }
            _ => {}
        }
    }

    if quote.is_some() {
        return Err("unterminated quoted selector segment".to_string());
    }
    if bracket_depth != 0 {
        return Err("unterminated attribute selector".to_string());
    }
    if paren_depth != 0 {
        return Err("unterminated selector function".to_string());
    }

    segments.push(&part[start..]);
    Ok(segments)
}

#[derive(Clone, Copy)]
enum LegadoCssContext<'a> {
    Document(&'a Html),
    Element(ElementRef<'a>),
}

fn apply_legado_css(input: &str, rule: &LegadoCssRule) -> RuleResult<Vec<String>> {
    if rule.is_empty() {
        return Ok(Vec::new());
    }

    let document = Html::parse_document(input);
    let mut contexts = vec![LegadoCssContext::Document(&document)];

    for step in rule.steps() {
        match step {
            LegadoCssStep::Select(selector) => {
                contexts = legado_css_select_contexts(&contexts, selector)?;
                if contexts.is_empty() {
                    return Ok(Vec::new());
                }
            }
            LegadoCssStep::Extract {
                selector,
                extraction,
            } => {
                let elements = if let Some(selector) = selector {
                    legado_css_select_elements(&contexts, selector)?
                } else {
                    legado_css_context_elements(&contexts)
                };
                return Ok(extract_legado_css_values(elements, extraction));
            }
        }
    }

    Ok(extract_legado_css_values(
        legado_css_context_elements(&contexts),
        &LegadoCssExtraction::Text,
    ))
}

fn legado_css_select_contexts<'a>(
    contexts: &[LegadoCssContext<'a>],
    selector: &str,
) -> RuleResult<Vec<LegadoCssContext<'a>>> {
    Ok(legado_css_select_elements(contexts, selector)?
        .into_iter()
        .map(LegadoCssContext::Element)
        .collect())
}

fn legado_css_select_elements<'a>(
    contexts: &[LegadoCssContext<'a>],
    selector: &str,
) -> RuleResult<Vec<ElementRef<'a>>> {
    let (base, index) = parse_legado_css_index(selector);
    let compiled = compile_legado_css_selector(base)?;
    let mut elements = Vec::new();

    for context in contexts {
        match context {
            LegadoCssContext::Document(document) => {
                elements.extend(document.select(&compiled));
            }
            LegadoCssContext::Element(element) => {
                elements.extend(element.select(&compiled));
            }
        }
    }

    if let Some(idx) = index {
        elements = apply_legado_css_index(elements, idx);
    }

    Ok(elements)
}

/// Parse the Legado dot-syntax index from the end of a selector.
///
/// Mirrors Legado `AnalyzeByJSoup.kt` `findIndexSet` dot-syntax branch
/// (line 482-506): trailing `.N` or `.-N` is a single-element index.
/// Returns `(base_selector, Some(index))` for `tag.p.1` -> `("tag.p", Some(1))`,
/// or `(selector, None)` when the trailing segment is non-numeric (e.g.
/// `div.item` -> the `.item` is a CSS class, not an index).
///
/// Only handles the simple `.N` / `.-N` select case. The `.!N` exclusion
/// form and the bracket `[N:M:K]` range form are not yet supported.
fn parse_legado_css_index(selector: &str) -> (&str, Option<i64>) {
    let bytes = selector.as_bytes();
    if bytes.is_empty() {
        return (selector, None);
    }

    // Collect trailing digits.
    let end = bytes.len();
    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }

    if start == end {
        return (selector, None); // no trailing digits
    }

    // Optional `-` before digits (negative index).
    let mut sign_start = start;
    if sign_start > 0 && bytes[sign_start - 1] == b'-' {
        sign_start -= 1;
    }

    // Must be preceded by `.` to be a Legado index separator.
    if sign_start > 0 && bytes[sign_start - 1] == b'.' {
        let index_str = &selector[sign_start..end];
        if let Ok(index) = index_str.parse::<i64>() {
            let base = &selector[..sign_start - 1];
            return (base, Some(index));
        }
    }

    (selector, None)
}

/// Apply a Legado single-index selector to the selected elements.
///
/// Mirrors Legado `AnalyzeByJSoup.kt` `ElementsSingle.getElementsSingle`
/// (line 324-402): positive index selects the Nth element; negative index
/// counts from the end. Out-of-range indices return an empty set.
fn apply_legado_css_index<'a>(
    elements: Vec<ElementRef<'a>>,
    index: i64,
) -> Vec<ElementRef<'a>> {
    let len = elements.len() as i64;
    if len == 0 {
        return Vec::new();
    }

    let resolved = if index >= 0 {
        index
    } else {
        index + len
    };

    if (0..len).contains(&resolved) {
        vec![elements[resolved as usize]]
    } else {
        Vec::new()
    }
}

fn compile_legado_css_selector(selector: &str) -> RuleResult<Selector> {
    let translated = translate_legado_css_shorthand(selector);
    Selector::parse(&translated).map_err(|err| RuleError::CssSelectorSyntax {
        selector: selector.to_string(),
        message: format!("{err:?}"),
    })
}

/// Translate Legado CSS shorthand prefixes to standard CSS selectors.
///
/// Mirrors Legado `AnalyzeByJSoup.kt` `ElementsSingle.getElementsSingle`
/// (lines 313-321): `beforeRule.split(".")` dispatches on `rules[0]`:
///   - `class.X` -> `getElementsByClass(X)` = CSS `.X`
///   - `tag.X`   -> `getElementsByTag(X)`   = CSS `X`
///   - `id.X`    -> `Evaluator.Id(X)`       = CSS `#X`
///
/// Only translates when the selector starts with `class.`, `tag.`, or `id.`
/// followed by a non-empty value. Other selectors pass through unchanged
/// (Legado's `else -> temp.select(beforeRule)` branch handles raw CSS).
fn translate_legado_css_shorthand(selector: &str) -> String {
    if let Some(rest) = selector.strip_prefix("class.") {
        if !rest.is_empty() {
            return format!(".{rest}");
        }
    }
    if let Some(rest) = selector.strip_prefix("tag.") {
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    if let Some(rest) = selector.strip_prefix("id.") {
        if !rest.is_empty() {
            return format!("#{rest}");
        }
    }
    selector.to_string()
}

fn legado_css_context_elements<'a>(contexts: &[LegadoCssContext<'a>]) -> Vec<ElementRef<'a>> {
    contexts
        .iter()
        .filter_map(|context| match context {
            LegadoCssContext::Document(_) => None,
            LegadoCssContext::Element(element) => Some(*element),
        })
        .collect()
}

fn extract_legado_css_values(
    elements: Vec<ElementRef<'_>>,
    extraction: &LegadoCssExtraction,
) -> Vec<String> {
    let mut output = Vec::new();

    for element in elements {
        match extraction {
            LegadoCssExtraction::Text => {
                output.push(element_text(&element));
            }
            LegadoCssExtraction::Html => {
                let value = element.inner_html();
                if !value.is_empty() {
                    output.push(value);
                }
            }
            LegadoCssExtraction::Attr(attr) => {
                if let Some(value) = element.value().attr(attr) {
                    output.push(value.to_string());
                }
            }
        }
    }

    output
}

fn apply_css(input: &str, rule: &CssRule) -> RuleResult<Vec<String>> {
    let document = Html::parse_document(input);
    let compat_selector = parse_css_compat_selector(&rule.selector).map_err(|message| {
        RuleError::CssSelectorSyntax {
            selector: rule.selector.clone(),
            message,
        }
    })?;
    let selector =
        Selector::parse(&compat_selector.selector).map_err(|err| RuleError::CssSelectorSyntax {
            selector: rule.selector.clone(),
            message: format!("{err:?}"),
        })?;
    let has_data_selector = compat_selector
        .has_data_filter
        .as_ref()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: format!("{err:?}"),
            })
        })
        .transpose()?;
    let has_text_selector = compat_selector
        .has_text_filter
        .as_ref()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: format!("{err:?}"),
            })
        })
        .transpose()?;
    let has_text_filter_regex = if let Some(filter) = &compat_selector.has_text_filter {
        if filter.text_filter.matcher == CssTextMatcher::Regex {
            Some(Regex::new(&filter.text_filter.value).map_err(|err| {
                RuleError::CssSelectorSyntax {
                    selector: rule.selector.clone(),
                    message: err.to_string(),
                }
            })?)
        } else {
            None
        }
    } else {
        None
    };
    let anchored_text_selector = compat_selector
        .anchored_text_filter
        .as_ref()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: format!("{err:?}"),
            })
        })
        .transpose()?;
    let anchored_text_filter_regex = if let Some(filter) = &compat_selector.anchored_text_filter {
        if filter.text_filter.matcher == CssTextMatcher::Regex {
            Some(Regex::new(&filter.text_filter.value).map_err(|err| {
                RuleError::CssSelectorSyntax {
                    selector: rule.selector.clone(),
                    message: err.to_string(),
                }
            })?)
        } else {
            None
        }
    } else {
        None
    };
    let text_filter_regex = if let Some(filter) = &compat_selector.text_filter {
        if filter.matcher == CssTextMatcher::Regex {
            Some(
                Regex::new(&filter.value).map_err(|err| RuleError::CssSelectorSyntax {
                    selector: rule.selector.clone(),
                    message: err.to_string(),
                })?,
            )
        } else {
            None
        }
    } else {
        None
    };
    let result_group_selectors = compat_selector
        .result_filter_groups
        .as_ref()
        .map(|groups| {
            groups
                .iter()
                .map(|group| {
                    Selector::parse(&group.selector).map_err(|err| RuleError::CssSelectorSyntax {
                        selector: rule.selector.clone(),
                        message: format!("{err:?}"),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;

    let collect_matching_elements = |selector: &Selector| {
        let mut elements = Vec::new();
        for element in document.select(selector) {
            if let Some(parent_filter) = compat_selector.parent_filter {
                let has_child_nodes = element_has_child_nodes(&element);
                match parent_filter {
                    CssParentFilter::HasChildren if !has_child_nodes => continue,
                    CssParentFilter::Empty if has_child_nodes => continue,
                    _ => {}
                }
            }
            if let Some(has_data_filter) = &compat_selector.has_data_filter {
                let Some(has_data_selector) = has_data_selector.as_ref() else {
                    continue;
                };
                if !element.select(has_data_selector).any(|descendant| {
                    let matches =
                        css_contains_data_matches(&descendant, &has_data_filter.data_filter.value);
                    match has_data_filter.data_filter.mode {
                        CssDataFilterMode::Contains => matches,
                        CssDataFilterMode::NotContains => !matches,
                    }
                }) {
                    continue;
                }
            }
            if let Some(has_text_filter) = &compat_selector.has_text_filter {
                let Some(has_text_selector) = has_text_selector.as_ref() else {
                    continue;
                };
                let has_matching_text = if has_text_filter.direct_child {
                    element.child_elements().any(|child| {
                        has_text_selector.matches(&child)
                            && css_element_text_filter_matches(
                                &child,
                                &has_text_filter.text_filter,
                                has_text_filter_regex.as_ref(),
                            )
                    })
                } else {
                    element.select(has_text_selector).any(|descendant| {
                        css_element_text_filter_matches(
                            &descendant,
                            &has_text_filter.text_filter,
                            has_text_filter_regex.as_ref(),
                        )
                    })
                };
                if !has_matching_text {
                    continue;
                }
            }
            if let Some(anchored_text_filter) = &compat_selector.anchored_text_filter {
                let Some(anchored_text_selector) = anchored_text_selector.as_ref() else {
                    continue;
                };
                let anchor_matches =
                    element
                        .ancestors()
                        .filter_map(ElementRef::wrap)
                        .any(|ancestor| {
                            anchored_text_selector.matches(&ancestor)
                                && match anchored_text_filter.mode {
                                    CssTextFilterMode::Contains => css_element_text_filter_matches(
                                        &ancestor,
                                        &anchored_text_filter.text_filter,
                                        anchored_text_filter_regex.as_ref(),
                                    ),
                                    CssTextFilterMode::NotContains => {
                                        !css_element_text_filter_matches(
                                            &ancestor,
                                            &anchored_text_filter.text_filter,
                                            anchored_text_filter_regex.as_ref(),
                                        )
                                    }
                                }
                        });
                if !anchor_matches {
                    continue;
                }
            }
            if let Some(data_filter) = &compat_selector.data_filter {
                let matches = css_contains_data_matches(&element, &data_filter.value);
                match data_filter.mode {
                    CssDataFilterMode::Contains if !matches => continue,
                    CssDataFilterMode::NotContains if matches => continue,
                    _ => {}
                }
            }

            if let Some(filter) = &compat_selector.text_filter {
                let haystack = css_element_text_filter_haystack(&element, filter);
                let matches =
                    css_text_filter_matches(&haystack, filter, text_filter_regex.as_ref());
                match compat_selector.text_filter_mode {
                    CssTextFilterMode::Contains if !matches => continue,
                    CssTextFilterMode::NotContains if matches => continue,
                    _ => {}
                }
            }

            elements.push(element);
        }
        elements
    };

    let elements = if let Some(groups) = &compat_selector.result_filter_groups {
        let Some(group_selectors) = result_group_selectors.as_ref() else {
            return Err(RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: "CSS result filter groups were not compiled".to_string(),
            });
        };
        let mut elements = Vec::new();
        for (group, selector) in groups.iter().zip(group_selectors) {
            let group_elements = collect_matching_elements(selector);
            elements.extend(apply_css_result_filters(group_elements, &group.filters));
        }
        elements
    } else {
        apply_css_result_filters(
            collect_matching_elements(&selector),
            &compat_selector.result_filters,
        )
    };
    let mut output = Vec::new();
    for element in elements {
        match &rule.extraction {
            CssExtraction::Text => {
                output.push(element_text(&element));
            }
            CssExtraction::Attr(attr) => {
                if attr == "html" {
                    let value = element.inner_html();
                    if !value.is_empty() {
                        output.push(value);
                    }
                } else if attr == "textNodes" {
                    let value = element_text(&element);
                    if !value.is_empty() {
                        output.push(value);
                    }
                } else if attr == "ownText" {
                    let value = normalize_text(&element_own_text(&element));
                    if !value.is_empty() {
                        output.push(value);
                    }
                } else if let Some(value) = element.value().attr(attr) {
                    output.push(value.to_string());
                }
            }
        }
    }

    Ok(output)
}

fn apply_css_result_filters<'a>(
    mut elements: Vec<ElementRef<'a>>,
    filters: &[CssResultFilter],
) -> Vec<ElementRef<'a>> {
    for filter in filters {
        let len = elements.len();
        elements = match filter.kind {
            CssResultFilterKind::Eq => {
                let index = resolve_css_result_index(filter.index, len);
                if index < 0 || index >= len as isize {
                    Vec::new()
                } else {
                    vec![elements[index as usize]]
                }
            }
            CssResultFilterKind::Lt => {
                let end = resolve_css_result_index(filter.index, len).clamp(0, len as isize);
                elements.into_iter().take(end as usize).collect()
            }
            CssResultFilterKind::Gt => {
                let start = resolve_css_result_index(filter.index, len);
                elements
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, element)| {
                        if (index as isize) > start {
                            Some(element)
                        } else {
                            None
                        }
                    })
                    .collect()
            }
        };
    }

    elements
}

fn resolve_css_result_index(index: isize, len: usize) -> isize {
    if index < 0 {
        len as isize + index
    } else {
        index
    }
}

fn css_text_filter_matches(text: &str, filter: &CssTextFilter, regex: Option<&Regex>) -> bool {
    match filter.matcher {
        CssTextMatcher::Contains => text.to_lowercase().contains(&filter.value.to_lowercase()),
        CssTextMatcher::ContainsCaseSensitive => text.contains(&filter.value),
        CssTextMatcher::Regex => regex.is_some_and(|regex| regex.is_match(text)),
    }
}

fn css_element_text_filter_matches(
    element: &ElementRef<'_>,
    filter: &CssTextFilter,
    regex: Option<&Regex>,
) -> bool {
    let haystack = css_element_text_filter_haystack(element, filter);

    css_text_filter_matches(&haystack, filter, regex)
}

fn css_element_text_filter_haystack(element: &ElementRef<'_>, filter: &CssTextFilter) -> String {
    match filter.scope {
        CssTextScope::Descendant => element_text(element),
        CssTextScope::Own => normalize_text(&element_own_text(element)),
        CssTextScope::WholeDescendant => element_whole_text(element),
        CssTextScope::WholeOwn => element_whole_own_text(element),
    }
}

fn element_has_child_nodes(element: &ElementRef<'_>) -> bool {
    element.children().next().is_some()
}

fn element_text(element: &ElementRef<'_>) -> String {
    let mut output = String::new();
    append_element_text(element, &mut output);
    normalize_text(&output)
}

fn element_whole_text(element: &ElementRef<'_>) -> String {
    element.text().collect::<Vec<_>>().join("")
}

fn append_element_text(element: &ElementRef<'_>, output: &mut String) {
    for child in element.children() {
        if let Some(text) = child.value().as_text() {
            output.push_str(text);
            continue;
        }

        let Some(child_element) = ElementRef::wrap(child) else {
            continue;
        };
        let name = child_element.value().name();
        if name.eq_ignore_ascii_case("br") {
            output.push(' ');
            continue;
        }

        let boundary = is_css_text_boundary_element(name);
        if boundary {
            output.push(' ');
        }
        append_element_text(&child_element, output);
        if boundary {
            output.push(' ');
        }
    }
}

fn is_css_text_boundary_element(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    )
}

fn css_contains_data_matches(element: &ElementRef<'_>, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    element.descendants().any(|node| {
        if let Some(comment) = node.value().as_comment() {
            return comment.to_lowercase().contains(&needle);
        }

        let Some(text) = node.value().as_text() else {
            return false;
        };
        if !node.ancestors().any(|ancestor| {
            ancestor
                .value()
                .as_element()
                .is_some_and(|element| matches!(element.name(), "script" | "style"))
        }) {
            return false;
        }

        text.to_lowercase().contains(&needle)
    })
}

fn element_own_text(element: &ElementRef<'_>) -> String {
    element
        .children()
        .filter_map(|child| child.value().as_text())
        .map(|text| &**text)
        .collect::<Vec<_>>()
        .join(" ")
}

fn element_whole_own_text(element: &ElementRef<'_>) -> String {
    element
        .children()
        .filter_map(|child| child.value().as_text())
        .map(|text| &**text)
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssCompatSelector {
    selector: String,
    text_filter: Option<CssTextFilter>,
    text_filter_mode: CssTextFilterMode,
    result_filters: Vec<CssResultFilter>,
    result_filter_groups: Option<Vec<CssResultFilterGroup>>,
    data_filter: Option<CssDataFilter>,
    has_data_filter: Option<CssHasDataFilter>,
    has_text_filter: Option<CssHasTextFilter>,
    anchored_text_filter: Option<CssAnchoredTextFilter>,
    parent_filter: Option<CssParentFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CssResultFilter {
    kind: CssResultFilterKind,
    index: isize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssResultFilterGroup {
    selector: String,
    filters: Vec<CssResultFilter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssResultFilterKind {
    Eq,
    Lt,
    Gt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssAnchoredTextFilter {
    selector: String,
    text_filter: CssTextFilter,
    mode: CssTextFilterMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssHasDataFilter {
    selector: String,
    data_filter: CssDataFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssHasTextFilter {
    selector: String,
    direct_child: bool,
    text_filter: CssTextFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssDataFilter {
    value: String,
    mode: CssDataFilterMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssDataFilterMode {
    Contains,
    NotContains,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssTextFilterMode {
    Contains,
    NotContains,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssTextFilter {
    value: String,
    scope: CssTextScope,
    matcher: CssTextMatcher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssTextScope {
    Descendant,
    Own,
    WholeDescendant,
    WholeOwn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssTextMatcher {
    Contains,
    ContainsCaseSensitive,
    Regex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssParentFilter {
    HasChildren,
    Empty,
}

fn parse_css_compat_selector(selector: &str) -> Result<CssCompatSelector, String> {
    let mut selector = selector.trim().to_string();
    let mut parent_filter = None;
    if let Some(base) = selector
        .strip_suffix(":not(:parent)")
        .map(|base| base.trim().to_string())
    {
        selector = base;
        parent_filter = Some(CssParentFilter::Empty);
    } else if let Some(base) = selector
        .strip_suffix(":parent")
        .map(|base| base.trim().to_string())
    {
        selector = base;
        parent_filter = Some(CssParentFilter::HasChildren);
    }

    let rewritten_selector = if parent_filter == Some(CssParentFilter::HasChildren) {
        strip_redundant_has_parent_filter(&selector)
    } else {
        None
    };
    if let Some(value) = rewritten_selector {
        selector = value;
    }

    let (rewritten_selector, result_filters, result_filter_groups) =
        extract_css_result_filter_groups(&selector)?;
    selector = rewritten_selector;

    let mut anchored_text_filter = None;
    if let Some((rewritten, filter)) = extract_css_anchored_text_filter(&selector)? {
        selector = rewritten;
        anchored_text_filter = Some(filter);
    }

    let mut has_data_filter = None;
    let mut has_text_filter = None;
    if let Some(suffix) = find_css_has_filter_suffix(&selector) {
        let close_index = selector.len() - 1;
        let base = selector[..suffix.open_index].trim();
        let inner = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
        if inner.is_empty() {
            return Err(format!("CSS {} requires selector", suffix.name));
        }
        if find_css_data_filter_suffix(inner).is_some() {
            has_data_filter = Some(parse_css_has_data_filter(inner)?);
        } else {
            has_text_filter = Some(parse_css_has_text_filter(inner)?);
        }
        selector = if base.is_empty() {
            "*".to_string()
        } else {
            base.to_string()
        };
    }

    let mut data_filter = None;
    if let Some(suffix) = find_css_not_data_filter_suffix(&selector) {
        let close_index = selector.len() - 2;
        let base = selector[..suffix.open_index].trim();
        let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
        if argument.is_empty() {
            return Err(format!("CSS {} requires text", suffix.name));
        }
        data_filter = Some(CssDataFilter {
            value: parse_css_text_filter_argument(argument, suffix.name)?,
            mode: CssDataFilterMode::NotContains,
        });
        selector = if base.is_empty() {
            "*".to_string()
        } else {
            base.to_string()
        };
    } else if let Some(suffix) = find_css_data_filter_suffix(&selector) {
        let close_index = selector.len() - 1;
        let base = selector[..suffix.open_index].trim();
        let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
        if argument.is_empty() {
            return Err(format!("CSS {} requires text", suffix.name));
        }
        data_filter = Some(CssDataFilter {
            value: parse_css_text_filter_argument(argument, suffix.name)?,
            mode: CssDataFilterMode::Contains,
        });
        selector = if base.is_empty() {
            "*".to_string()
        } else {
            base.to_string()
        };
    }

    let mut text_filter_mode = CssTextFilterMode::Contains;
    let text_filter = if let Some(suffix) = find_css_not_text_filter_suffix(&selector) {
        text_filter_mode = CssTextFilterMode::NotContains;
        Some((suffix, selector.len() - 2))
    } else {
        find_css_text_filter_suffix(&selector).map(|suffix| (suffix, selector.len() - 1))
    };

    let Some((suffix, close_index)) = text_filter else {
        return Ok(CssCompatSelector {
            selector: if selector.is_empty() {
                "*".to_string()
            } else {
                selector
            },
            text_filter: None,
            text_filter_mode,
            result_filters,
            result_filter_groups,
            data_filter,
            has_data_filter,
            has_text_filter,
            anchored_text_filter,
            parent_filter,
        });
    };

    let base = selector[..suffix.open_index].trim();
    let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
    if argument.is_empty() {
        return Err(format!("CSS {} requires text", suffix.name));
    }

    let filter_value = parse_css_text_filter_argument(argument, suffix.name)?;
    let selector = if base.is_empty() { "*" } else { base };

    Ok(CssCompatSelector {
        selector: selector.to_string(),
        text_filter: Some(CssTextFilter {
            value: filter_value,
            scope: suffix.scope,
            matcher: suffix.matcher,
        }),
        text_filter_mode,
        result_filters,
        result_filter_groups,
        data_filter,
        has_data_filter,
        has_text_filter,
        anchored_text_filter,
        parent_filter,
    })
}

fn extract_css_result_filter_groups(
    selector: &str,
) -> Result<
    (
        String,
        Vec<CssResultFilter>,
        Option<Vec<CssResultFilterGroup>>,
    ),
    String,
> {
    let groups = split_css_selector_groups(selector);
    if groups.len() <= 1 {
        let (rewritten, filters) = extract_css_result_filters(selector)?;
        return Ok((rewritten, filters, None));
    }

    let mut has_group_filter = false;
    let mut rewritten_groups = Vec::new();
    let mut result_groups = Vec::new();
    for group in groups {
        let (rewritten, filters) = extract_css_result_filters(group.trim())?;
        if !filters.is_empty() {
            has_group_filter = true;
        }
        let selector = if rewritten.trim().is_empty() {
            "*".to_string()
        } else {
            rewritten.trim().to_string()
        };
        rewritten_groups.push(selector.clone());
        result_groups.push(CssResultFilterGroup { selector, filters });
    }

    let rewritten = rewritten_groups.join(",");
    if has_group_filter {
        Ok((rewritten, Vec::new(), Some(result_groups)))
    } else {
        Ok((rewritten, Vec::new(), None))
    }
}

fn split_css_selector_groups(selector: &str) -> Vec<&str> {
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut groups = Vec::new();
    let mut start = 0usize;

    for (index, value) in selector.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ',' if bracket_depth == 0 && paren_depth == 0 => {
                groups.push(&selector[start..index]);
                start = index + value.len_utf8();
            }
            _ => {}
        }
    }

    groups.push(&selector[start..]);
    groups
}

fn extract_css_result_filters(selector: &str) -> Result<(String, Vec<CssResultFilter>), String> {
    let mut rewritten = String::new();
    let mut filters = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut index = 0usize;

    while index < selector.len() {
        let value = selector[index..]
            .chars()
            .next()
            .ok_or_else(|| "invalid CSS selector".to_string())?;

        if let Some(active_quote) = quote {
            rewritten.push(value);
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            index += value.len_utf8();
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            rewritten.push(value);
            index += value.len_utf8();
            continue;
        }

        if value == ':' && bracket_depth == 0 && paren_depth == 0 {
            if let Some((filter, consumed)) = css_result_filter_keyword(&selector[index..]) {
                filters.push(filter);
                index += consumed;
                continue;
            } else if let Some((kind, prefix, name)) = css_result_filter_prefix(&selector[index..])
            {
                let argument_start = index + prefix.len();
                let close_index = matching_css_function_close(selector, argument_start)
                    .ok_or_else(|| format!("unterminated CSS {name} argument in `{selector}`"))?;
                let argument = selector[argument_start..close_index].trim();
                if argument.is_empty() {
                    return Err(format!("CSS {name} requires index"));
                }
                let index_value = argument
                    .parse::<isize>()
                    .map_err(|_| format!("CSS {name} index must be an integer"))?;
                filters.push(CssResultFilter {
                    kind,
                    index: index_value,
                });
                index = close_index + 1;
                continue;
            }
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }
        rewritten.push(value);
        index += value.len_utf8();
    }

    Ok((rewritten, filters))
}

fn css_result_filter_keyword(selector: &str) -> Option<(CssResultFilter, usize)> {
    if css_result_keyword_matches(selector, ":first") {
        Some((
            CssResultFilter {
                kind: CssResultFilterKind::Eq,
                index: 0,
            },
            ":first".len(),
        ))
    } else if css_result_keyword_matches(selector, ":last") {
        Some((
            CssResultFilter {
                kind: CssResultFilterKind::Eq,
                index: -1,
            },
            ":last".len(),
        ))
    } else {
        None
    }
}

fn css_result_keyword_matches(selector: &str, keyword: &str) -> bool {
    if !selector.starts_with(keyword) {
        return false;
    }

    match selector[keyword.len()..].chars().next() {
        Some(value) => !is_css_identifier_char(value),
        None => true,
    }
}

fn is_css_identifier_char(value: char) -> bool {
    value == '-' || value == '_' || value.is_ascii_alphanumeric()
}

fn css_result_filter_prefix(
    selector: &str,
) -> Option<(CssResultFilterKind, &'static str, &'static str)> {
    if selector.starts_with(":eq(") {
        Some((CssResultFilterKind::Eq, ":eq(", ":eq()"))
    } else if selector.starts_with(":lt(") {
        Some((CssResultFilterKind::Lt, ":lt(", ":lt()"))
    } else if selector.starts_with(":gt(") {
        Some((CssResultFilterKind::Gt, ":gt(", ":gt()"))
    } else {
        None
    }
}

fn extract_css_anchored_text_filter(
    selector: &str,
) -> Result<Option<(String, CssAnchoredTextFilter)>, String> {
    if let Some(suffix) = find_css_not_text_filter_function(selector) {
        let argument_start = suffix.open_index + suffix.prefix_len;
        let close_index = matching_css_function_close(selector, argument_start)
            .ok_or_else(|| format!("unterminated CSS {} argument in `{selector}`", suffix.name))?;
        let outer_close_index = close_index + 1;
        if selector.as_bytes().get(outer_close_index) != Some(&b')') {
            return Err(format!(
                "unterminated CSS {} argument in `{selector}`",
                suffix.name
            ));
        }

        let tail = &selector[outer_close_index + 1..];
        if tail.trim().is_empty() {
            return Ok(None);
        }

        let base = selector[..suffix.open_index].trim_end();
        if base.is_empty() {
            return Ok(None);
        }

        let argument = selector[argument_start..close_index].trim();
        if argument.is_empty() {
            return Err(format!("CSS {} requires text", suffix.name));
        }

        return Ok(Some((
            format!("{base}{tail}"),
            CssAnchoredTextFilter {
                selector: base.to_string(),
                text_filter: CssTextFilter {
                    value: parse_css_text_filter_argument(argument, suffix.name)?,
                    scope: suffix.scope,
                    matcher: suffix.matcher,
                },
                mode: CssTextFilterMode::NotContains,
            },
        )));
    }

    let Some(suffix) = find_css_text_filter_function(selector) else {
        return Ok(None);
    };

    let argument_start = suffix.open_index + suffix.prefix_len;
    let close_index = matching_css_function_close(selector, argument_start)
        .ok_or_else(|| format!("unterminated CSS {} argument in `{selector}`", suffix.name))?;
    let tail = &selector[close_index + 1..];
    if tail.trim().is_empty() {
        return Ok(None);
    }

    let base = selector[..suffix.open_index].trim_end();
    if base.is_empty() {
        return Ok(None);
    }

    let argument = selector[argument_start..close_index].trim();
    if argument.is_empty() {
        return Err(format!("CSS {} requires text", suffix.name));
    }

    Ok(Some((
        format!("{base}{tail}"),
        CssAnchoredTextFilter {
            selector: base.to_string(),
            text_filter: CssTextFilter {
                value: parse_css_text_filter_argument(argument, suffix.name)?,
                scope: suffix.scope,
                matcher: suffix.matcher,
            },
            mode: CssTextFilterMode::Contains,
        },
    )))
}

fn parse_css_has_data_filter(selector: &str) -> Result<CssHasDataFilter, String> {
    let Some(suffix) = find_css_data_filter_suffix(selector) else {
        return Err(format!(
            "CSS :has() compatibility requires :containsData() in `{selector}`"
        ));
    };

    let close_index = selector.len() - 1;
    let base = selector[..suffix.open_index].trim();
    let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
    if argument.is_empty() {
        return Err(format!("CSS {} requires text", suffix.name));
    }

    Ok(CssHasDataFilter {
        selector: if base.is_empty() {
            "*".to_string()
        } else {
            base.to_string()
        },
        data_filter: CssDataFilter {
            value: parse_css_text_filter_argument(argument, suffix.name)?,
            mode: CssDataFilterMode::Contains,
        },
    })
}

fn parse_css_has_text_filter(selector: &str) -> Result<CssHasTextFilter, String> {
    let Some(suffix) = find_css_text_filter_suffix(selector) else {
        return Err(format!(
            "CSS :has() compatibility requires a supported text filter in `{selector}`"
        ));
    };

    let close_index = selector.len() - 1;
    let base = selector[..suffix.open_index].trim();
    let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
    if argument.is_empty() {
        return Err(format!("CSS {} requires text", suffix.name));
    }

    let direct_child = base.starts_with('>');
    let selector = if direct_child {
        base.trim_start_matches('>').trim()
    } else {
        base
    };

    Ok(CssHasTextFilter {
        selector: if selector.is_empty() {
            "*".to_string()
        } else {
            selector.to_string()
        },
        direct_child,
        text_filter: CssTextFilter {
            value: parse_css_text_filter_argument(argument, suffix.name)?,
            scope: suffix.scope,
            matcher: suffix.matcher,
        },
    })
}

fn strip_redundant_has_parent_filter(selector: &str) -> Option<String> {
    let open_index = find_css_function_suffix(selector, ":has(")?;
    let inner_start = open_index + ":has(".len();
    let close_index = matching_css_function_close(selector, inner_start)?;
    let inner = selector[inner_start..close_index].trim();
    let child_selector = inner.strip_suffix(":parent")?.trim();
    if !child_selector.starts_with('>') {
        return None;
    }

    let remainder = selector[close_index + 1..].trim();
    if !css_selector_text_equivalent(remainder, child_selector) {
        return None;
    }

    let base = selector[..open_index].trim_end();
    if base.is_empty() {
        return Some(child_selector.to_string());
    }

    Some(format!("{base} {child_selector}"))
}

fn css_selector_text_equivalent(left: &str, right: &str) -> bool {
    left.chars()
        .filter(|value| !value.is_whitespace())
        .eq(right.chars().filter(|value| !value.is_whitespace()))
}

fn matching_css_function_close(selector: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 1usize;

    for (offset, value) in selector[start..].char_indices() {
        let index = start + offset;
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' if bracket_depth == 0 => paren_depth += 1,
            ')' if bracket_depth == 0 => {
                paren_depth = paren_depth.saturating_sub(1);
                if paren_depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CssTextFilterSuffix {
    open_index: usize,
    prefix_len: usize,
    scope: CssTextScope,
    matcher: CssTextMatcher,
    name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CssDataFilterSuffix {
    open_index: usize,
    prefix_len: usize,
    name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CssHasFilterSuffix {
    open_index: usize,
    prefix_len: usize,
    name: &'static str,
}

fn find_css_has_filter_suffix(selector: &str) -> Option<CssHasFilterSuffix> {
    if !selector.ends_with(')') {
        return None;
    }

    find_css_function_suffix(selector, ":has(").map(|open_index| CssHasFilterSuffix {
        open_index,
        prefix_len: ":has(".len(),
        name: ":has()",
    })
}

fn find_css_data_filter_suffix(selector: &str) -> Option<CssDataFilterSuffix> {
    if !selector.ends_with(')') {
        return None;
    }

    find_css_function_suffix(selector, ":containsData(").map(|open_index| CssDataFilterSuffix {
        open_index,
        prefix_len: ":containsData(".len(),
        name: ":containsData()",
    })
}

fn find_css_not_data_filter_suffix(selector: &str) -> Option<CssDataFilterSuffix> {
    if !selector.ends_with("))") {
        return None;
    }

    find_css_function_suffix(selector, ":not(:containsData(").map(|open_index| {
        CssDataFilterSuffix {
            open_index,
            prefix_len: ":not(:containsData(".len(),
            name: ":not(:containsData())",
        }
    })
}

fn find_css_text_filter_suffix(selector: &str) -> Option<CssTextFilterSuffix> {
    if !selector.ends_with(')') {
        return None;
    }

    find_css_text_filter_function(selector)
}

fn find_css_not_text_filter_suffix(selector: &str) -> Option<CssTextFilterSuffix> {
    if !selector.ends_with("))") {
        return None;
    }

    find_css_not_text_filter_function(selector)
}

fn find_css_not_text_filter_function(selector: &str) -> Option<CssTextFilterSuffix> {
    if let Some(open_index) = find_css_function_suffix(selector, ":not(:containsWholeOwnText(") {
        return Some(CssTextFilterSuffix {
            open_index,
            prefix_len: ":not(:containsWholeOwnText(".len(),
            scope: CssTextScope::WholeOwn,
            matcher: CssTextMatcher::ContainsCaseSensitive,
            name: ":not(:containsWholeOwnText())",
        });
    }
    if let Some(open_index) = find_css_function_suffix(selector, ":not(:containsWholeText(") {
        return Some(CssTextFilterSuffix {
            open_index,
            prefix_len: ":not(:containsWholeText(".len(),
            scope: CssTextScope::WholeDescendant,
            matcher: CssTextMatcher::ContainsCaseSensitive,
            name: ":not(:containsWholeText())",
        });
    }
    if let Some(open_index) = find_css_function_suffix(selector, ":not(:containsOwn(") {
        return Some(CssTextFilterSuffix {
            open_index,
            prefix_len: ":not(:containsOwn(".len(),
            scope: CssTextScope::Own,
            matcher: CssTextMatcher::Contains,
            name: ":not(:containsOwn())",
        });
    }
    if let Some(open_index) = find_css_function_suffix(selector, ":not(:contains(") {
        return Some(CssTextFilterSuffix {
            open_index,
            prefix_len: ":not(:contains(".len(),
            scope: CssTextScope::Descendant,
            matcher: CssTextMatcher::Contains,
            name: ":not(:contains())",
        });
    }
    if let Some(open_index) = find_css_function_suffix(selector, ":not(:matchesWholeOwnText(") {
        return Some(CssTextFilterSuffix {
            open_index,
            prefix_len: ":not(:matchesWholeOwnText(".len(),
            scope: CssTextScope::WholeOwn,
            matcher: CssTextMatcher::Regex,
            name: ":not(:matchesWholeOwnText())",
        });
    }
    if let Some(open_index) = find_css_function_suffix(selector, ":not(:matchesWholeText(") {
        return Some(CssTextFilterSuffix {
            open_index,
            prefix_len: ":not(:matchesWholeText(".len(),
            scope: CssTextScope::WholeDescendant,
            matcher: CssTextMatcher::Regex,
            name: ":not(:matchesWholeText())",
        });
    }
    if let Some(open_index) = find_css_function_suffix(selector, ":not(:matchesOwn(") {
        return Some(CssTextFilterSuffix {
            open_index,
            prefix_len: ":not(:matchesOwn(".len(),
            scope: CssTextScope::Own,
            matcher: CssTextMatcher::Regex,
            name: ":not(:matchesOwn())",
        });
    }
    if let Some(open_index) = find_css_function_suffix(selector, ":not(:matches(") {
        return Some(CssTextFilterSuffix {
            open_index,
            prefix_len: ":not(:matches(".len(),
            scope: CssTextScope::Descendant,
            matcher: CssTextMatcher::Regex,
            name: ":not(:matches())",
        });
    }

    None
}

fn find_css_text_filter_function(selector: &str) -> Option<CssTextFilterSuffix> {
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut candidate = None;

    for (index, value) in selector.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ':' if bracket_depth == 0 && paren_depth == 0 => {
                if selector[index..].starts_with(":containsWholeOwnText(") {
                    candidate = Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":containsWholeOwnText(".len(),
                        scope: CssTextScope::WholeOwn,
                        matcher: CssTextMatcher::ContainsCaseSensitive,
                        name: ":containsWholeOwnText()",
                    });
                } else if selector[index..].starts_with(":containsWholeText(") {
                    candidate = Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":containsWholeText(".len(),
                        scope: CssTextScope::WholeDescendant,
                        matcher: CssTextMatcher::ContainsCaseSensitive,
                        name: ":containsWholeText()",
                    });
                } else if selector[index..].starts_with(":containsOwn(") {
                    candidate = Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":containsOwn(".len(),
                        scope: CssTextScope::Own,
                        matcher: CssTextMatcher::Contains,
                        name: ":containsOwn()",
                    });
                } else if selector[index..].starts_with(":contains(") {
                    candidate = Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":contains(".len(),
                        scope: CssTextScope::Descendant,
                        matcher: CssTextMatcher::Contains,
                        name: ":contains()",
                    });
                } else if selector[index..].starts_with(":matchesWholeOwnText(") {
                    candidate = Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":matchesWholeOwnText(".len(),
                        scope: CssTextScope::WholeOwn,
                        matcher: CssTextMatcher::Regex,
                        name: ":matchesWholeOwnText()",
                    });
                } else if selector[index..].starts_with(":matchesWholeText(") {
                    candidate = Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":matchesWholeText(".len(),
                        scope: CssTextScope::WholeDescendant,
                        matcher: CssTextMatcher::Regex,
                        name: ":matchesWholeText()",
                    });
                } else if selector[index..].starts_with(":matchesOwn(") {
                    candidate = Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":matchesOwn(".len(),
                        scope: CssTextScope::Own,
                        matcher: CssTextMatcher::Regex,
                        name: ":matchesOwn()",
                    });
                } else if selector[index..].starts_with(":matches(") {
                    candidate = Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":matches(".len(),
                        scope: CssTextScope::Descendant,
                        matcher: CssTextMatcher::Regex,
                        name: ":matches()",
                    });
                }
            }
            _ => {}
        }
    }

    candidate
}

fn find_css_function_suffix(selector: &str, function: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut candidate = None;

    for (index, value) in selector.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if value == '\\' {
                escaped = true;
            } else if value == active_quote {
                quote = None;
            }
            continue;
        }

        if value == '\'' || value == '"' {
            quote = Some(value);
            continue;
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ':' if bracket_depth == 0
                && paren_depth == 0
                && selector[index..].starts_with(function) =>
            {
                candidate = Some(index);
            }
            _ => {}
        }
    }

    candidate
}

fn parse_css_text_filter_argument(argument: &str, name: &str) -> Result<String, String> {
    let chars = argument.chars().collect::<Vec<_>>();
    if matches!(chars.first(), Some('\'' | '"')) {
        let quote = chars[0];
        let mut output = String::new();
        let mut index = 1;

        while index < chars.len() {
            match chars[index] {
                '\\' if index + 1 < chars.len() => {
                    index += 1;
                    output.push(chars[index]);
                    index += 1;
                }
                current if current == quote => {
                    index += 1;
                    if index == chars.len() {
                        return Ok(output);
                    }
                    return Err(format!(
                        "unexpected trailing CSS {name} argument text in `{argument}`"
                    ));
                }
                current => {
                    output.push(current);
                    index += 1;
                }
            }
        }

        return Err(format!("unterminated CSS {name} argument in `{argument}`"));
    }

    Ok(argument.to_string())
}

fn normalize_text(text: &str) -> String {
    let mut output = String::new();
    let mut pending_space = false;

    for value in text.chars() {
        if value.is_whitespace() || value == '\u{a0}' {
            pending_space = true;
        } else {
            if pending_space && !output.is_empty() {
                output.push(' ');
            }
            output.push(value);
            pending_space = false;
        }
    }

    output
}

fn apply_xpath(input: &str, rule: &XPathRule) -> RuleResult<Vec<String>> {
    validate_xpath_namespaces(rule)?;

    let package = sxd_document::parser::parse(input).map_err(|err| RuleError::XPathInputParse {
        message: err.to_string(),
    })?;
    let document = package.as_document();
    let factory = Factory::new();
    let xpath = factory
        .build(&rule.expression)
        .map_err(|err| RuleError::XPathSyntax {
            expression: rule.expression.clone(),
            message: err.to_string(),
        })?
        .ok_or_else(|| RuleError::XPathSyntax {
            expression: rule.expression.clone(),
            message: "empty expression".to_string(),
        })?;
    let mut context = Context::new();
    for (prefix, uri) in &rule.namespaces {
        context.set_namespace(prefix, uri);
    }

    match xpath
        .evaluate(&context, document.root())
        .map_err(|err| RuleError::XPathEvaluation {
            expression: rule.expression.clone(),
            message: err.to_string(),
        })? {
        XPathValue::Nodeset(nodes) => Ok(nodes
            .document_order()
            .into_iter()
            .map(|node| node.string_value())
            .collect()),
        XPathValue::String(value) => Ok(vec![value]),
        XPathValue::Number(value) => Ok(vec![value.to_string()]),
        XPathValue::Boolean(value) => Ok(vec![value.to_string()]),
    }
}

fn validate_xpath_namespaces(rule: &XPathRule) -> RuleResult<()> {
    for prefix in xpath_prefixes(&rule.expression) {
        if !rule
            .namespaces
            .iter()
            .any(|(registered, _)| registered == &prefix)
        {
            return Err(RuleError::XPathEvaluation {
                expression: rule.expression.clone(),
                message: format!("namespace prefix `{prefix}` is not registered"),
            });
        }
    }

    Ok(())
}

fn xpath_prefixes(expression: &str) -> Vec<String> {
    let chars = expression.chars().collect::<Vec<_>>();
    let mut prefixes = Vec::new();
    let mut quote = None;

    for (index, value) in chars.iter().enumerate() {
        if let Some(active_quote) = quote {
            if *value == active_quote {
                quote = None;
            }
            continue;
        }

        if *value == '\'' || *value == '"' {
            quote = Some(*value);
            continue;
        }

        if *value != ':' || chars.get(index + 1) == Some(&':') || index == 0 {
            continue;
        }

        let mut start = index;
        while start > 0 && is_xpath_name_char(chars[start - 1]) {
            start -= 1;
        }

        if start == index || chars.get(index - 1) == Some(&':') {
            continue;
        }

        let prefix = chars[start..index].iter().collect::<String>();
        if !prefixes.contains(&prefix) {
            prefixes.push(prefix);
        }
    }

    prefixes
}

fn is_xpath_name_char(value: char) -> bool {
    value == '_' || value == '-' || value == '.' || value.is_ascii_alphanumeric()
}

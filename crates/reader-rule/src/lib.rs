//! Non-JS rule execution primitives for Reader-Core.
//!
//! This crate owns the first native rule semantics before the protocol/runtime
//! layer is ready. The public API is intentionally local to rule execution:
//! callers provide source text and a list of rule steps, and receive a flat list
//! of string results.

use regex::{Regex, RegexBuilder};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
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
        let mut context = LegadoRuleContext::new();
        self.execute_legado_css_with_context(input, rule, &mut context)
    }

    pub fn execute_legado_css_with_context(
        &self,
        input: &str,
        rule: &str,
        context: &mut LegadoRuleContext,
    ) -> RuleResult<RuleOutput> {
        let (rule, put_bindings) = split_legado_put_bindings(rule)?;
        for binding in put_bindings {
            let value = self.evaluate_legado_put_binding(input, &binding.rule, context)?;
            context.put_variable(binding.key, value);
        }

        let (rule, has_get) = materialize_legado_get_variables(&rule, context);
        if has_get && !rule.contains("{{") {
            return Ok(RuleOutput::new(apply_legado_literal_rule(&rule)));
        }

        if let Some(path) = strip_legado_json_path_prefix(&rule) {
            return Ok(RuleOutput::new(apply_json_path(
                input,
                &JsonPathRule::new(path),
            )?));
        }

        if let Some(expression) = strip_legado_xpath_prefix(&rule) {
            return Ok(RuleOutput::new(apply_xpath(
                input,
                &XPathRule::new(expression),
            )?));
        }

        if let Some(values) = evaluate_legado_rule_embedded_template(input, &rule, context)? {
            return Ok(RuleOutput::new(values));
        }

        if let Some(values) = evaluate_legado_css_embedded_template(input, &rule, context)? {
            return Ok(RuleOutput::new(values));
        }

        let rule = LegadoCssRule::parse(&rule)?;
        self.execute_legado_css_rule(input, &rule)
    }

    fn evaluate_legado_put_binding(
        &self,
        input: &str,
        rule: &str,
        context: &mut LegadoRuleContext,
    ) -> RuleResult<String> {
        let rule = normalize_legado_put_value_rule(input, rule);
        let output = self.execute_legado_css_with_context(input, &rule, context)?;
        Ok(output.into_values().join("\n"))
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

    pub fn execute_legado_css_list_items_with_context(
        &self,
        input: &str,
        rule: &str,
        context: &mut LegadoRuleContext,
    ) -> RuleResult<RuleOutput> {
        let (rule, put_bindings) = split_legado_put_bindings(rule)?;
        for binding in put_bindings {
            let value = self.evaluate_legado_put_binding(input, &binding.rule, context)?;
            context.put_variable(binding.key, value);
        }

        let (rule, _) = materialize_legado_get_variables(&rule, context);
        let rule = LegadoCssRule::parse(&rule)?;
        Ok(RuleOutput::new(apply_legado_css_list_items(input, &rule)?))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LegadoRuleContext {
    variables: HashMap<String, String>,
}

impl LegadoRuleContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_variable(&mut self, key: impl Into<String>, value: impl Into<String>) -> String {
        let value = value.into();
        self.variables.insert(key.into(), value.clone());
        value
    }

    pub fn get_variable(&self, key: &str) -> Option<&str> {
        self.variables.get(key).map(String::as_str)
    }

    pub fn variables(&self) -> &HashMap<String, String> {
        &self.variables
    }
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
    combination: Option<LegadoCssCombination>,
    branches: Vec<Vec<LegadoCssStep>>,
    selector_mode: LegadoCssSelectorMode,
    replacement: Option<LegadoRuleReplacement>,
}

impl LegadoCssRule {
    pub fn parse(rule: &str) -> RuleResult<Self> {
        parse_legado_css_rule(rule).map_err(|message| RuleError::LegadoCssSyntax {
            rule: rule.to_string(),
            message,
        })
    }

    pub fn missing() -> Self {
        Self {
            steps: Vec::new(),
            combination: None,
            branches: Vec::new(),
            selector_mode: LegadoCssSelectorMode::Default,
            replacement: None,
        }
    }

    pub fn steps(&self) -> &[LegadoCssStep] {
        &self.steps
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.branches.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegadoCssCombination {
    And,
    Or,
    Zip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegadoCssSelectorMode {
    Default,
    CssSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegadoRuleReplacement {
    pattern: String,
    replacement: String,
    first_match_only: bool,
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
    TextNodes,
    OwnText,
    Html,
    All,
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
    if find_json_path_template_marker(path).is_some() {
        return evaluate_json_path_expression_without_replacement(value, path);
    }

    let (path, replacement) = split_legado_rule_replacement(path);
    let values = evaluate_json_path_expression_without_replacement(value, path)?;
    Ok(apply_legado_rule_replacement(values, replacement.as_ref()))
}

fn evaluate_json_path_expression_without_replacement(
    value: &JsonValue,
    path: &str,
) -> Result<Vec<String>, String> {
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

    if let Some(value) = evaluate_json_path_embedded_template(value, path)? {
        return Ok(value);
    }

    evaluate_json_path_rule(value, path)
}

fn evaluate_json_path_embedded_template(
    value: &JsonValue,
    template: &str,
) -> Result<Option<Vec<String>>, String> {
    let mut output = String::new();
    let mut literal_start = 0usize;
    let mut search_start = 0usize;
    let mut saw_template = false;
    let mut replaced = false;

    while let Some(relative_marker) = find_json_path_template_marker(&template[search_start..]) {
        let marker = relative_marker.shifted(search_start);
        saw_template = true;
        let Ok(end) = find_json_path_template_end(template, marker) else {
            search_start = marker.rule_start;
            continue;
        };
        let rule = &template[marker.rule_start..end.rule_end];

        let Ok(values) = evaluate_json_path_expression(value, rule) else {
            search_start = end.template_end;
            continue;
        };
        if values.is_empty() {
            search_start = end.template_end;
            continue;
        }

        output.push_str(&template[literal_start..marker.start]);
        output.push_str(&values.join("\n"));
        literal_start = end.template_end;
        search_start = literal_start;
        replaced = true;
    }

    if !replaced {
        if saw_template {
            return Ok(Some(Vec::new()));
        }
        return Ok(None);
    }

    output.push_str(&template[literal_start..]);
    Ok(Some(vec![output]))
}

fn strip_legado_json_path_prefix(rule: &str) -> Option<&str> {
    let trimmed = rule.trim();

    trimmed
        .get(.."@JSON:".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("@JSON:"))
        .then(|| trimmed["@JSON:".len()..].trim())
        .or_else(|| {
            (trimmed == "$" || trimmed.starts_with("$.") || trimmed.starts_with("$["))
                .then_some(trimmed)
        })
}

fn strip_legado_xpath_prefix(rule: &str) -> Option<&str> {
    let trimmed = rule.trim();

    trimmed
        .get(.."@XPATH:".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("@XPATH:"))
        .then(|| trimmed["@XPATH:".len()..].trim())
        .or_else(|| trimmed.starts_with('/').then_some(trimmed))
}

#[derive(Clone, Copy)]
struct JsonPathTemplateMarker {
    start: usize,
    rule_start: usize,
    delimiter: JsonPathTemplateDelimiter,
}

impl JsonPathTemplateMarker {
    fn shifted(self, offset: usize) -> Self {
        Self {
            start: self.start + offset,
            rule_start: self.rule_start + offset,
            delimiter: self.delimiter,
        }
    }
}

#[derive(Clone, Copy)]
enum JsonPathTemplateDelimiter {
    SingleBrace,
    DoubleBrace,
}

struct JsonPathTemplateEnd {
    rule_end: usize,
    template_end: usize,
}

fn find_json_path_template_marker(value: &str) -> Option<JsonPathTemplateMarker> {
    let mut quote = None;
    let mut escaped = false;
    let mut index = 0usize;

    while index < value.len() {
        let current = value[index..].chars().next()?;

        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if current == '\\' {
                escaped = true;
            } else if current == active_quote {
                quote = None;
            }
            index += current.len_utf8();
            continue;
        }

        if current == '\'' || current == '"' {
            quote = Some(current);
            index += current.len_utf8();
            continue;
        }

        if value[index..].starts_with("{{$.") {
            return Some(JsonPathTemplateMarker {
                start: index,
                rule_start: index + "{{".len(),
                delimiter: JsonPathTemplateDelimiter::DoubleBrace,
            });
        }

        if value[index..].starts_with("{$.") {
            return Some(JsonPathTemplateMarker {
                start: index,
                rule_start: index + "{".len(),
                delimiter: JsonPathTemplateDelimiter::SingleBrace,
            });
        }

        index += current.len_utf8();
    }

    None
}

fn find_json_path_template_end(
    template: &str,
    marker: JsonPathTemplateMarker,
) -> Result<JsonPathTemplateEnd, String> {
    match marker.delimiter {
        JsonPathTemplateDelimiter::SingleBrace => {
            find_single_brace_json_path_template_end(template, marker)
        }
        JsonPathTemplateDelimiter::DoubleBrace => {
            find_double_brace_json_path_template_end(template, marker)
        }
    }
}

fn find_single_brace_json_path_template_end(
    template: &str,
    marker: JsonPathTemplateMarker,
) -> Result<JsonPathTemplateEnd, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut brace_depth = 0usize;
    let mut index = marker.start;

    while index < template.len() {
        let value = template[index..]
            .chars()
            .next()
            .ok_or_else(|| "invalid JSONPath embedded rule template".to_string())?;

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

        match value {
            '{' => brace_depth += 1,
            '}' => {
                if brace_depth == 0 {
                    return Err(format!("unmatched `}}` in JSONPath template `{template}`"));
                }
                brace_depth -= 1;
                if brace_depth == 0 {
                    return Ok(JsonPathTemplateEnd {
                        rule_end: index,
                        template_end: index + '}'.len_utf8(),
                    });
                }
            }
            _ => {}
        }

        index += value.len_utf8();
    }

    Err(format!(
        "unterminated JSONPath embedded rule template in `{template}`"
    ))
}

fn find_double_brace_json_path_template_end(
    template: &str,
    marker: JsonPathTemplateMarker,
) -> Result<JsonPathTemplateEnd, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut brace_depth = 0usize;
    let mut index = marker.rule_start;

    while index < template.len() {
        let value = template[index..]
            .chars()
            .next()
            .ok_or_else(|| "invalid JSONPath embedded rule template".to_string())?;

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

        if brace_depth == 0 && template[index..].starts_with("}}") {
            return Ok(JsonPathTemplateEnd {
                rule_end: index,
                template_end: index + "}}".len(),
            });
        }

        match value {
            '{' => brace_depth += 1,
            '}' => {
                if brace_depth == 0 {
                    return Err(format!("unmatched `}}` in JSONPath template `{template}`"));
                }
                brace_depth -= 1;
            }
            _ => {}
        }

        index += value.len_utf8();
    }

    Err(format!(
        "unterminated JSONPath embedded rule template in `{template}`"
    ))
}

fn evaluate_legado_css_embedded_template(
    input: &str,
    template: &str,
    context: &mut LegadoRuleContext,
) -> RuleResult<Option<Vec<String>>> {
    let (template, replacement) = split_legado_embedded_template_replacement(template);
    let mut output = String::new();
    let mut literal_start = 0usize;
    let mut search_start = 0usize;
    let mut saw_template = false;
    let mut replaced = false;

    while let Some(relative_marker) =
        find_legado_css_embedded_template_marker(&template[search_start..])
    {
        let marker = relative_marker.shifted(search_start);
        saw_template = true;
        let end = find_legado_css_embedded_template_end(template, marker).map_err(|message| {
            RuleError::LegadoCssSyntax {
                rule: template.to_string(),
                message,
            }
        })?;
        let rule = &template[marker.rule_start..end];
        let values = RuleEngine::new()
            .execute_legado_css_with_context(input, rule, context)?
            .into_values();

        if values.is_empty() {
            search_start = end + "}}".len();
            continue;
        }

        output.push_str(&template[literal_start..marker.start]);
        output.push_str(&values.join("\n"));
        literal_start = end + "}}".len();
        search_start = literal_start;
        replaced = true;
    }

    if !replaced {
        if saw_template {
            return Ok(Some(Vec::new()));
        }
        return Ok(None);
    }

    output.push_str(&template[literal_start..]);
    Ok(Some(apply_legado_rule_replacement(
        vec![output],
        replacement.as_ref(),
    )))
}

fn evaluate_legado_rule_embedded_template(
    input: &str,
    template: &str,
    context: &mut LegadoRuleContext,
) -> RuleResult<Option<Vec<String>>> {
    let (template, replacement) = split_legado_embedded_template_replacement(template);
    let mut output = String::new();
    let mut literal_start = 0usize;
    let mut search_start = 0usize;
    let mut saw_template = false;
    let mut replaced = false;

    while let Some(relative_marker) =
        find_legado_rule_embedded_template_marker(&template[search_start..])
    {
        let marker = relative_marker.shifted(search_start);
        saw_template = true;
        let end = find_legado_css_embedded_template_end(template, marker).map_err(|message| {
            RuleError::LegadoCssSyntax {
                rule: template.to_string(),
                message,
            }
        })?;
        let rule = &template[marker.rule_start..end];
        let values = RuleEngine::new()
            .execute_legado_css_with_context(input, rule, context)?
            .into_values();

        if values.is_empty() {
            search_start = end + "}}".len();
            continue;
        }

        output.push_str(&template[literal_start..marker.start]);
        output.push_str(&values.join("\n"));
        literal_start = end + "}}".len();
        search_start = literal_start;
        replaced = true;
    }

    if !replaced {
        if saw_template {
            return Ok(Some(Vec::new()));
        }
        return Ok(None);
    }

    output.push_str(&template[literal_start..]);
    Ok(Some(apply_legado_rule_replacement(
        vec![output],
        replacement.as_ref(),
    )))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegadoPutBinding {
    key: String,
    rule: String,
}

fn split_legado_put_bindings(rule: &str) -> RuleResult<(String, Vec<LegadoPutBinding>)> {
    let mut output = String::new();
    let mut bindings = Vec::new();
    let mut index = 0usize;

    while let Some(relative_start) = find_ascii_case_insensitive(&rule[index..], "@put:{") {
        let start = index + relative_start;
        let body_start = start + "@put:".len();
        let Some(relative_end) = rule[body_start..].find('}') else {
            return Err(RuleError::LegadoCssSyntax {
                rule: rule.to_string(),
                message: "unterminated @put binding".to_string(),
            });
        };
        let end = body_start + relative_end;
        output.push_str(&rule[index..start]);
        bindings.extend(parse_legado_put_bindings(rule, &rule[body_start + 1..end])?);
        index = end + '}'.len_utf8();
    }

    output.push_str(&rule[index..]);
    Ok((output, bindings))
}

fn parse_legado_put_bindings(rule: &str, body: &str) -> RuleResult<Vec<LegadoPutBinding>> {
    let mut bindings = Vec::new();
    for entry in split_legado_put_entries(body).map_err(|message| RuleError::LegadoCssSyntax {
        rule: rule.to_string(),
        message,
    })? {
        let Some(colon) = find_legado_put_entry_colon(entry) else {
            return Err(RuleError::LegadoCssSyntax {
                rule: rule.to_string(),
                message: format!("invalid @put binding `{entry}`"),
            });
        };
        let key = strip_legado_put_token(&entry[..colon]);
        if key.is_empty() {
            return Err(RuleError::LegadoCssSyntax {
                rule: rule.to_string(),
                message: format!("empty @put key in `{entry}`"),
            });
        }
        let value = strip_legado_put_token(&entry[colon + ':'.len_utf8()..]);
        bindings.push(LegadoPutBinding { key, rule: value });
    }
    Ok(bindings)
}

fn split_legado_put_entries(body: &str) -> Result<Vec<&str>, String> {
    let mut entries = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut start = 0usize;
    let mut index = 0usize;

    while index < body.len() {
        let value = body[index..]
            .chars()
            .next()
            .ok_or_else(|| "invalid @put body".to_string())?;

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

        if value == ',' {
            let entry = body[start..index].trim();
            if !entry.is_empty() {
                entries.push(entry);
            }
            start = index + value.len_utf8();
        }
        index += value.len_utf8();
    }

    if quote.is_some() {
        return Err("unterminated quote in @put body".to_string());
    }

    let entry = body[start..].trim();
    if !entry.is_empty() {
        entries.push(entry);
    }
    Ok(entries)
}

fn find_legado_put_entry_colon(entry: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut index = 0usize;

    while index < entry.len() {
        let value = entry[index..].chars().next()?;

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
        } else if value == ':' {
            return Some(index);
        }

        index += value.len_utf8();
    }

    None
}

fn strip_legado_put_token(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let first = value.as_bytes()[0] as char;
        let last = value.as_bytes()[value.len() - 1] as char;
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return unescape_legado_quoted_put_token(&value[1..value.len() - 1]);
        }
    }
    value.to_string()
}

fn unescape_legado_quoted_put_token(value: &str) -> String {
    let mut output = String::new();
    let mut escaped = false;
    for item in value.chars() {
        if escaped {
            output.push(item);
            escaped = false;
        } else if item == '\\' {
            escaped = true;
        } else {
            output.push(item);
        }
    }
    if escaped {
        output.push('\\');
    }
    output
}

fn normalize_legado_put_value_rule(input: &str, rule: &str) -> String {
    let rule = rule.trim();
    if is_bare_legado_json_key(rule) && serde_json::from_str::<JsonValue>(input).is_ok() {
        format!("$.{rule}")
    } else {
        rule.to_string()
    }
}

fn is_bare_legado_json_key(rule: &str) -> bool {
    let mut chars = rule.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|value| value == '_' || value.is_ascii_alphanumeric())
}

fn materialize_legado_get_variables(rule: &str, context: &LegadoRuleContext) -> (String, bool) {
    let mut output = String::new();
    let mut index = 0usize;
    let mut replaced = false;

    while let Some(relative_start) = find_ascii_case_insensitive(&rule[index..], "@get:{") {
        let start = index + relative_start;
        let key_start = start + "@get:{".len();
        let Some(relative_end) = rule[key_start..].find('}') else {
            break;
        };
        let end = key_start + relative_end;
        output.push_str(&rule[index..start]);
        output.push_str(
            context
                .get_variable(rule[key_start..end].trim())
                .unwrap_or(""),
        );
        index = end + '}'.len_utf8();
        replaced = true;
    }

    if !replaced {
        return (rule.to_string(), false);
    }

    output.push_str(&rule[index..]);
    (output, true)
}

fn apply_legado_literal_rule(rule: &str) -> Vec<String> {
    let (base_rule, replacement) = split_legado_embedded_template_replacement(rule);
    apply_legado_rule_replacement(vec![base_rule.to_string()], replacement.as_ref())
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack.char_indices().find_map(|(index, _)| {
        haystack[index..]
            .get(..needle.len())
            .filter(|candidate| candidate.eq_ignore_ascii_case(needle))
            .map(|_| index)
    })
}

fn split_legado_embedded_template_replacement(rule: &str) -> (&str, Option<LegadoRuleReplacement>) {
    let separators = find_legado_embedded_template_replacement_separators(rule);
    let Some(first_separator) = separators.first().copied() else {
        return (rule, None);
    };

    let base_rule = rule[..first_separator].trim();
    let pattern_start = first_separator + "##".len();
    let pattern_end = separators.get(1).copied().unwrap_or(rule.len());
    let pattern = &rule[pattern_start..pattern_end];
    if pattern.is_empty() {
        return (base_rule, None);
    }

    let replacement = separators
        .get(1)
        .map(|separator| {
            let replacement_start = separator + "##".len();
            let replacement_end = separators.get(2).copied().unwrap_or(rule.len());
            &rule[replacement_start..replacement_end]
        })
        .unwrap_or_default();

    (
        base_rule,
        Some(LegadoRuleReplacement {
            pattern: pattern.to_string(),
            replacement: replacement.to_string(),
            first_match_only: separators.len() > 2,
        }),
    )
}

fn find_legado_embedded_template_replacement_separators(rule: &str) -> Vec<usize> {
    let mut separators = Vec::new();
    let mut index = 0usize;

    while index < rule.len() {
        if rule[index..].starts_with("{{") {
            let marker = LegadoCssEmbeddedTemplateMarker {
                start: index,
                rule_start: index + "{{".len(),
            };
            if let Ok(end) = find_legado_css_embedded_template_end(rule, marker) {
                index = end + "}}".len();
                continue;
            }
        }

        if rule[index..].starts_with("##") {
            separators.push(index);
            index += "##".len();
            continue;
        }

        let value = rule[index..]
            .chars()
            .next()
            .expect("index is inside rule bounds");
        index += value.len_utf8();
    }

    separators
}

fn find_legado_rule_embedded_template_marker(
    value: &str,
) -> Option<LegadoCssEmbeddedTemplateMarker> {
    let mut search_start = 0usize;
    while let Some(relative_start) = value[search_start..].find("{{") {
        let start = search_start + relative_start;
        let rule_start = start + "{{".len();
        let rule = &value[rule_start..];

        if rule.starts_with("//")
            || rule.starts_with("$.")
            || rule.starts_with("$[")
            || rule
                .get(.."@XPATH:".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("@XPATH:"))
            || rule
                .get(.."@JSON:".len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("@JSON:"))
        {
            return Some(LegadoCssEmbeddedTemplateMarker { start, rule_start });
        }

        search_start = rule_start;
    }

    None
}

#[derive(Clone, Copy)]
struct LegadoCssEmbeddedTemplateMarker {
    start: usize,
    rule_start: usize,
}

impl LegadoCssEmbeddedTemplateMarker {
    fn shifted(self, offset: usize) -> Self {
        Self {
            start: self.start + offset,
            rule_start: self.rule_start + offset,
        }
    }
}

fn find_legado_css_embedded_template_marker(
    value: &str,
) -> Option<LegadoCssEmbeddedTemplateMarker> {
    let start = value.find("{{@")?;
    let rule_start = if value[start..].starts_with("{{@@") {
        start + "{{@@".len()
    } else {
        start + "{{@".len()
    };

    Some(LegadoCssEmbeddedTemplateMarker { start, rule_start })
}

fn find_legado_css_embedded_template_end(
    template: &str,
    marker: LegadoCssEmbeddedTemplateMarker,
) -> Result<usize, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut index = marker.rule_start;

    while index < template.len() {
        let value = template[index..]
            .chars()
            .next()
            .ok_or_else(|| "invalid Legado CSS embedded rule template".to_string())?;

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

        if template[index..].starts_with("}}") {
            return Ok(index);
        }

        index += value.len_utf8();
    }

    Err(format!(
        "unterminated Legado CSS embedded rule template in `{template}`"
    ))
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
            '{' => brace_depth += 1,
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
    let rule = rule.trim();
    if rule.is_empty() {
        return Ok(LegadoCssRule::missing());
    }

    let (rule, replacement) = split_legado_rule_replacement(rule);
    let mut parsed = parse_legado_css_rule_without_replacement(rule)?;
    parsed.replacement = replacement;
    Ok(parsed)
}

fn parse_legado_css_rule_without_replacement(rule: &str) -> Result<LegadoCssRule, String> {
    let rule = rule.trim();
    if rule.is_empty() {
        return Ok(LegadoCssRule::missing());
    }

    if let Some(default_rule) = strip_legado_default_css_escape_prefix(rule) {
        return parse_legado_css_rule_without_replacement(default_rule);
    }

    if let Some(source_rule) = strip_legado_css_source_prefix(rule) {
        return parse_legado_css_source_rule(source_rule);
    }

    if let Some(branches) = split_legado_css_top_level_operator(rule, "&&")? {
        if let Some(branches) = parse_legado_css_default_and_branches(branches)? {
            return Ok(LegadoCssRule {
                steps: Vec::new(),
                combination: Some(LegadoCssCombination::And),
                branches,
                selector_mode: LegadoCssSelectorMode::Default,
                replacement: None,
            });
        }
    }

    if let Some(branches) = split_legado_css_top_level_operator(rule, "||")? {
        return Ok(LegadoCssRule {
            steps: Vec::new(),
            combination: Some(LegadoCssCombination::Or),
            branches: parse_legado_css_branches(branches, LegadoCssSelectorMode::Default)?,
            selector_mode: LegadoCssSelectorMode::Default,
            replacement: None,
        });
    }

    if let Some(branches) = split_legado_css_top_level_operator(rule, "%%")? {
        return Ok(LegadoCssRule {
            steps: Vec::new(),
            combination: Some(LegadoCssCombination::Zip),
            branches: parse_legado_css_branches(branches, LegadoCssSelectorMode::Default)?,
            selector_mode: LegadoCssSelectorMode::Default,
            replacement: None,
        });
    }

    Ok(LegadoCssRule {
        steps: parse_legado_css_steps(rule, LegadoCssSelectorMode::Default)?,
        combination: None,
        branches: Vec::new(),
        selector_mode: LegadoCssSelectorMode::Default,
        replacement: None,
    })
}

fn split_legado_rule_replacement(rule: &str) -> (&str, Option<LegadoRuleReplacement>) {
    let parts = rule.split("##").collect::<Vec<_>>();
    if parts.len() == 1 {
        return (rule, None);
    }

    let base_rule = parts[0].trim();
    let pattern = parts.get(1).copied().unwrap_or_default();
    if pattern.is_empty() {
        return (base_rule, None);
    }

    (
        base_rule,
        Some(LegadoRuleReplacement {
            pattern: pattern.to_string(),
            replacement: parts.get(2).copied().unwrap_or_default().to_string(),
            first_match_only: parts.len() > 3,
        }),
    )
}

fn parse_legado_css_branches(
    branches: Vec<&str>,
    selector_mode: LegadoCssSelectorMode,
) -> Result<Vec<Vec<LegadoCssStep>>, String> {
    branches
        .into_iter()
        .map(|branch| parse_legado_css_steps(branch, selector_mode))
        .collect::<Result<Vec<_>, _>>()
}

fn parse_legado_css_default_and_branches(
    branches: Vec<&str>,
) -> Result<Option<Vec<Vec<LegadoCssStep>>>, String> {
    let parsed = branches
        .into_iter()
        .map(|branch| parse_legado_css_steps(branch, LegadoCssSelectorMode::Default))
        .collect::<Result<Vec<_>, _>>()?;

    if parsed.iter().all(|steps| {
        matches!(
            steps.last(),
            Some(LegadoCssStep::Extract {
                selector: _,
                extraction: _
            })
        )
    }) {
        Ok(Some(parsed))
    } else {
        Ok(None)
    }
}

fn strip_legado_default_css_escape_prefix(rule: &str) -> Option<&str> {
    rule.starts_with("@@").then(|| rule[2..].trim())
}

fn strip_legado_css_source_prefix(rule: &str) -> Option<&str> {
    if rule
        .get(.."@CSS:".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("@CSS:"))
    {
        Some(rule[5..].trim())
    } else if rule
        .get(.."CSS:".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("CSS:"))
    {
        Some(rule[4..].trim())
    } else {
        None
    }
}

fn parse_legado_css_source_rule(rule: &str) -> Result<LegadoCssRule, String> {
    let rule = rule.trim();
    if rule.is_empty() {
        return Ok(LegadoCssRule::missing());
    }

    if let Some(operator) = find_legado_css_top_level_operator(rule, &["&&", "||", "%%"])? {
        let branches = split_legado_css_top_level_operator(rule, operator)?
            .expect("operator found by top-level scan must split into branches");
        let combination = match operator {
            "&&" => LegadoCssCombination::And,
            "||" => LegadoCssCombination::Or,
            "%%" => LegadoCssCombination::Zip,
            _ => unreachable!(),
        };
        return Ok(LegadoCssRule {
            steps: Vec::new(),
            combination: Some(combination),
            branches: parse_legado_css_branches(branches, LegadoCssSelectorMode::CssSource)?,
            selector_mode: LegadoCssSelectorMode::CssSource,
            replacement: None,
        });
    }

    Ok(LegadoCssRule {
        steps: parse_legado_css_steps(rule, LegadoCssSelectorMode::CssSource)?,
        combination: None,
        branches: Vec::new(),
        selector_mode: LegadoCssSelectorMode::CssSource,
        replacement: None,
    })
}

fn find_legado_css_top_level_operator<'a>(
    rule: &str,
    operators: &[&'a str],
) -> Result<Option<&'a str>, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut index = 0usize;

    while index < rule.len() {
        let value = rule[index..]
            .chars()
            .next()
            .ok_or_else(|| "invalid Legado CSS combination expression".to_string())?;

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
            if let Some(operator) = operators
                .iter()
                .copied()
                .find(|operator| rule[index..].starts_with(operator))
            {
                return Ok(Some(operator));
            }
        }

        match value {
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            _ => {}
        }

        index += value.len_utf8();
    }

    if quote.is_some() || bracket_depth != 0 || paren_depth != 0 {
        return Err(format!("unterminated Legado CSS source rule in `{rule}`"));
    }

    Ok(None)
}

fn parse_legado_css_steps(
    rule: &str,
    selector_mode: LegadoCssSelectorMode,
) -> Result<Vec<LegadoCssStep>, String> {
    let parts = split_legado_css_pipeline(rule)?;
    let mut steps = Vec::new();

    for (index, part) in parts.iter().enumerate() {
        let segment_steps = parse_legado_css_segment(part, selector_mode)?;
        if segment_steps
            .iter()
            .any(|step| matches!(step, LegadoCssStep::Extract { .. }))
            && index + 1 != parts.len()
        {
            return Err("extraction step must be the final pipeline segment".to_string());
        }
        steps.extend(segment_steps);
    }

    Ok(steps)
}

fn parse_legado_css_segment(
    part: &str,
    selector_mode: LegadoCssSelectorMode,
) -> Result<Vec<LegadoCssStep>, String> {
    let part = part.trim();
    if part.is_empty() {
        return Err("empty pipeline segment".to_string());
    }

    if selector_mode == LegadoCssSelectorMode::Default {
        let segments = split_legado_css_at_segments(part)?;
        if segments.len() > 2 {
            return parse_legado_css_at_selector_chain(&segments);
        }
    }

    Ok(vec![parse_legado_css_step(part)?])
}

fn split_legado_css_at_segments(part: &str) -> Result<Vec<&str>, String> {
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
            start = index + value.len_utf8();
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

fn parse_legado_css_at_selector_chain(segments: &[&str]) -> Result<Vec<LegadoCssStep>, String> {
    let extraction = segments
        .last()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| "missing extraction after `@`".to_string())?;
    let selector_segments = &segments[..segments.len() - 1];
    let extraction_selector = selector_segments
        .last()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .ok_or_else(|| "missing selector before nested `@` extraction".to_string())?;

    let mut steps = Vec::new();
    for selector in &selector_segments[..selector_segments.len() - 1] {
        let selector = selector.trim();
        if selector.is_empty() {
            return Err("empty nested `@` selector segment".to_string());
        }
        steps.push(LegadoCssStep::Select(selector.to_string()));
    }
    steps.push(LegadoCssStep::Extract {
        selector: Some(extraction_selector.to_string()),
        extraction: parse_legado_css_extraction(extraction),
    });

    Ok(steps)
}

fn split_legado_css_top_level_operator<'a>(
    rule: &'a str,
    operator: &str,
) -> Result<Option<Vec<&'a str>>, String> {
    let operator_marker = operator
        .chars()
        .next()
        .ok_or_else(|| "Legado CSS combination operator must not be empty".to_string())?;
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut branches = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;

    while index < rule.len() {
        let value = rule[index..]
            .chars()
            .next()
            .ok_or_else(|| "invalid Legado CSS combination expression".to_string())?;

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
            && rule[index..].starts_with(operator)
            && bracket_depth == 0
            && paren_depth == 0
        {
            let branch = rule[start..index].trim();
            if branch.is_empty() {
                return Err(format!(
                    "empty Legado CSS `{operator}` combination branch in `{rule}`"
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
            _ => {}
        }

        index += value.len_utf8();
    }

    if branches.is_empty() {
        return Ok(None);
    }

    if quote.is_some() || bracket_depth != 0 || paren_depth != 0 {
        return Err(format!(
            "unterminated Legado CSS `{operator}` combination branch in `{rule}`"
        ));
    }

    let branch = rule[start..].trim();
    if branch.is_empty() {
        return Err(format!(
            "empty Legado CSS `{operator}` combination branch in `{rule}`"
        ));
    }
    branches.push(branch);

    Ok(Some(branches))
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

fn parse_legado_css_step(part: &str) -> Result<LegadoCssStep, String> {
    let part = part.trim();
    if part.is_empty() {
        return Err("empty pipeline segment".to_string());
    }

    if let Some(separator) = find_legado_css_extraction_separator(part)? {
        let selector = part[..separator].trim();
        let extraction = part[separator + '@'.len_utf8()..].trim();
        if extraction.is_empty() {
            return Err("missing extraction after `@`".to_string());
        }

        return Ok(LegadoCssStep::Extract {
            selector: if selector.is_empty() {
                None
            } else {
                Some(selector.to_string())
            },
            extraction: parse_legado_css_extraction(extraction),
        });
    }

    Ok(LegadoCssStep::Select(part.to_string()))
}

fn parse_legado_css_extraction(extraction: &str) -> LegadoCssExtraction {
    match extraction {
        "text" => LegadoCssExtraction::Text,
        "textNodes" => LegadoCssExtraction::TextNodes,
        "ownText" => LegadoCssExtraction::OwnText,
        "html" => LegadoCssExtraction::Html,
        "all" => LegadoCssExtraction::All,
        _ => LegadoCssExtraction::Attr(extraction.to_string()),
    }
}

fn find_legado_css_extraction_separator(part: &str) -> Result<Option<usize>, String> {
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

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
            return Ok(Some(index));
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

    Ok(None)
}

#[derive(Clone, Copy)]
enum LegadoCssContext<'a> {
    Document(&'a Html),
    Element(ElementRef<'a>),
}

fn apply_legado_css(input: &str, rule: &LegadoCssRule) -> RuleResult<Vec<String>> {
    if rule.is_empty() {
        if rule.replacement.is_some() {
            return Ok(apply_legado_rule_replacement(
                vec![input.to_string()],
                rule.replacement.as_ref(),
            ));
        }
        return Ok(Vec::new());
    }

    let values = match rule.combination {
        Some(LegadoCssCombination::And) => {
            let mut output = Vec::new();
            for branch in &rule.branches {
                let results = apply_legado_css_steps(input, branch, rule.selector_mode)?;
                if !results.is_empty() {
                    output.extend(results);
                }
            }
            output
        }
        Some(LegadoCssCombination::Or) => {
            let mut output = Vec::new();
            for branch in &rule.branches {
                let results = apply_legado_css_steps(input, branch, rule.selector_mode)?;
                if !results.is_empty() {
                    output = results;
                    break;
                }
            }
            output
        }
        Some(LegadoCssCombination::Zip) => {
            let mut branch_results = Vec::new();
            for branch in &rule.branches {
                let results = apply_legado_css_steps(input, branch, rule.selector_mode)?;
                if !results.is_empty() {
                    branch_results.push(results);
                }
            }
            zip_legado_css_combination_results(branch_results)
        }
        None => apply_legado_css_steps(input, rule.steps(), rule.selector_mode)?,
    };

    Ok(apply_legado_rule_replacement(
        values,
        rule.replacement.as_ref(),
    ))
}

fn apply_legado_css_list_items(input: &str, rule: &LegadoCssRule) -> RuleResult<Vec<String>> {
    if rule.is_empty() {
        return Ok(Vec::new());
    }

    let values = match rule.combination {
        Some(LegadoCssCombination::And) => {
            let mut output = Vec::new();
            for branch in &rule.branches {
                let results = apply_legado_css_list_item_steps(input, branch, rule.selector_mode)?;
                if !results.is_empty() {
                    output.extend(results);
                }
            }
            output
        }
        Some(LegadoCssCombination::Or) => {
            let mut output = Vec::new();
            for branch in &rule.branches {
                let results = apply_legado_css_list_item_steps(input, branch, rule.selector_mode)?;
                if !results.is_empty() {
                    output = results;
                    break;
                }
            }
            output
        }
        Some(LegadoCssCombination::Zip) => {
            let mut branch_results = Vec::new();
            for branch in &rule.branches {
                let results = apply_legado_css_list_item_steps(input, branch, rule.selector_mode)?;
                if !results.is_empty() {
                    branch_results.push(results);
                }
            }
            zip_legado_css_combination_results(branch_results)
        }
        None => apply_legado_css_list_item_steps(input, rule.steps(), rule.selector_mode)?,
    };

    Ok(apply_legado_rule_replacement(
        values,
        rule.replacement.as_ref(),
    ))
}

fn apply_legado_css_list_item_steps(
    input: &str,
    steps: &[LegadoCssStep],
    selector_mode: LegadoCssSelectorMode,
) -> RuleResult<Vec<String>> {
    if steps.is_empty() {
        return Ok(Vec::new());
    }

    let document = Html::parse_document(input);
    let mut contexts = vec![LegadoCssContext::Document(&document)];

    for step in steps {
        match step {
            LegadoCssStep::Select(selector) => {
                contexts = legado_css_select_contexts(&contexts, selector, selector_mode)?;
                if contexts.is_empty() {
                    return Ok(Vec::new());
                }
            }
            LegadoCssStep::Extract {
                selector,
                extraction,
            } => {
                let elements = if let Some(selector) = selector {
                    legado_css_select_elements(&contexts, selector, selector_mode)?
                } else {
                    legado_css_context_elements(&contexts)
                };
                return Ok(extract_legado_css_values(elements, extraction));
            }
        }
    }

    Ok(legado_css_context_elements(&contexts)
        .into_iter()
        .map(|element| element.html())
        .filter(|html| !html.is_empty())
        .collect())
}

fn apply_legado_rule_replacement(
    values: Vec<String>,
    replacement: Option<&LegadoRuleReplacement>,
) -> Vec<String> {
    let Some(replacement) = replacement else {
        return values;
    };
    let regex_pattern = legado_regex_pattern_for_rust(&replacement.pattern);
    let regex = Regex::new(&regex_pattern).ok();
    let regex_replacement = legado_regex_replacement_for_rust(&replacement.replacement);

    values
        .into_iter()
        .map(|value| {
            apply_legado_rule_replacement_to_value(
                value,
                replacement,
                regex.as_ref(),
                &regex_replacement,
            )
        })
        .collect()
}

fn apply_legado_rule_replacement_to_value(
    value: String,
    replacement: &LegadoRuleReplacement,
    regex: Option<&Regex>,
    regex_replacement: &str,
) -> String {
    if replacement.first_match_only {
        if let Some(regex) = regex {
            return regex
                .find(&value)
                .map(|matched| {
                    regex
                        .replacen(matched.as_str(), 1, regex_replacement)
                        .into_owned()
                })
                .unwrap_or_default();
        }
        return replacement.replacement.clone();
    }

    if let Some(regex) = regex {
        regex.replace_all(&value, regex_replacement).into_owned()
    } else {
        value.replace(&replacement.pattern, &replacement.replacement)
    }
}

fn legado_regex_pattern_for_rust(pattern: &str) -> String {
    let mut output = String::with_capacity(pattern.len());
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut in_char_class = false;

    while index < chars.len() {
        let value = chars[index];

        if value == '\\' {
            if chars.get(index + 1) == Some(&'h') {
                if in_char_class {
                    output.push_str(r"\t\p{Zs}");
                } else {
                    output.push_str(r"[\t\p{Zs}]");
                }
                index += 2;
                continue;
            }

            output.push(value);
            index += 1;
            continue;
        }

        match value {
            '[' if !in_char_class => in_char_class = true,
            ']' if in_char_class => in_char_class = false,
            _ => {}
        }
        output.push(value);
        index += 1;
    }

    output
}

fn legado_regex_replacement_for_rust(replacement: &str) -> String {
    let mut output = String::with_capacity(replacement.len());
    let chars = replacement.chars().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] != '$' {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        let digit_start = index + 1;
        let mut digit_end = digit_start;
        while digit_end < chars.len() && chars[digit_end].is_ascii_digit() {
            digit_end += 1;
        }

        if digit_end == digit_start {
            output.push('$');
            index += 1;
            continue;
        }

        output.push_str("${");
        for value in &chars[digit_start..digit_end] {
            output.push(*value);
        }
        output.push('}');
        index = digit_end;
    }

    output
}

fn apply_legado_css_steps(
    input: &str,
    steps: &[LegadoCssStep],
    selector_mode: LegadoCssSelectorMode,
) -> RuleResult<Vec<String>> {
    let document = Html::parse_document(input);
    let mut contexts = vec![LegadoCssContext::Document(&document)];

    for step in steps {
        match step {
            LegadoCssStep::Select(selector) => {
                contexts = legado_css_select_contexts(&contexts, selector, selector_mode)?;
                if contexts.is_empty() {
                    return Ok(Vec::new());
                }
            }
            LegadoCssStep::Extract {
                selector,
                extraction,
            } => {
                let elements = if let Some(selector) = selector {
                    legado_css_select_elements(&contexts, selector, selector_mode)?
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

fn zip_legado_css_combination_results(results: Vec<Vec<String>>) -> Vec<String> {
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

fn legado_css_select_contexts<'a>(
    contexts: &[LegadoCssContext<'a>],
    selector: &str,
    selector_mode: LegadoCssSelectorMode,
) -> RuleResult<Vec<LegadoCssContext<'a>>> {
    Ok(
        legado_css_select_elements(contexts, selector, selector_mode)?
            .into_iter()
            .map(LegadoCssContext::Element)
            .collect(),
    )
}

fn legado_css_select_elements<'a>(
    contexts: &[LegadoCssContext<'a>],
    selector: &str,
    selector_mode: LegadoCssSelectorMode,
) -> RuleResult<Vec<ElementRef<'a>>> {
    match selector_mode {
        LegadoCssSelectorMode::Default => select_legado_jsoup_default_elements(contexts, selector),
        LegadoCssSelectorMode::CssSource => select_css_compat_elements(contexts, selector),
    }
}

fn select_legado_jsoup_default_elements<'a>(
    contexts: &[LegadoCssContext<'a>],
    selector: &str,
) -> RuleResult<Vec<ElementRef<'a>>> {
    let (base_selector, index_filter) = parse_legado_jsoup_index_filter(selector);
    let elements = if base_selector.is_empty() || is_legado_jsoup_children_selector(base_selector) {
        legado_jsoup_direct_children(contexts)
    } else if let Some(shorthand) = parse_legado_jsoup_shorthand_selector(base_selector) {
        let candidates = legado_jsoup_default_candidates(contexts)?;
        candidates
            .into_iter()
            .filter(|element| legado_jsoup_shorthand_matches(element, shorthand))
            .collect()
    } else {
        select_css_compat_elements(contexts, base_selector)?
    };

    Ok(apply_legado_jsoup_index_filter(elements, index_filter))
}

#[derive(Clone, Copy)]
enum LegadoJsoupShorthandSelector<'a> {
    Class(&'a str),
    Tag(&'a str),
    Id(&'a str),
    OwnText(&'a str),
}

fn parse_legado_jsoup_shorthand_selector(
    selector: &str,
) -> Option<LegadoJsoupShorthandSelector<'_>> {
    let mut parts = selector.split('.');
    let kind = parts.next()?;
    let value = parts.next()?;
    if value.is_empty() {
        return None;
    }

    match kind {
        "class" => Some(LegadoJsoupShorthandSelector::Class(value)),
        "tag" => Some(LegadoJsoupShorthandSelector::Tag(value)),
        "id" => Some(LegadoJsoupShorthandSelector::Id(value)),
        "text" => Some(LegadoJsoupShorthandSelector::OwnText(value)),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LegadoJsoupIndexMode {
    Select,
    Exclude,
}

struct LegadoJsoupIndexFilter {
    mode: LegadoJsoupIndexMode,
    selectors: Vec<LegadoJsoupIndexSelector>,
}

enum LegadoJsoupIndexSelector {
    Index(isize),
    Range {
        start: Option<isize>,
        end: Option<isize>,
        step: isize,
    },
}

fn parse_legado_jsoup_index_filter(selector: &str) -> (&str, Option<LegadoJsoupIndexFilter>) {
    let selector = selector.trim();

    if let Some((base, filter)) = parse_legado_jsoup_bracket_index_filter(selector) {
        return (base, Some(filter));
    }

    if let Some((base, filter)) = parse_legado_jsoup_legacy_index_filter(selector) {
        return (base, Some(filter));
    }

    (selector, None)
}

fn parse_legado_jsoup_bracket_index_filter(
    selector: &str,
) -> Option<(&str, LegadoJsoupIndexFilter)> {
    if !selector.ends_with(']') {
        return None;
    }

    let bracket = selector.rfind('[')?;
    let base = selector[..bracket].trim();

    let content = selector[bracket + 1..selector.len() - 1].trim();
    let (mode, content) = if let Some(content) = content.strip_prefix('!') {
        (LegadoJsoupIndexMode::Exclude, content.trim())
    } else {
        (LegadoJsoupIndexMode::Select, content)
    };
    if content.is_empty() {
        return None;
    }

    let selectors = content
        .split(',')
        .map(parse_legado_jsoup_bracket_index_token)
        .collect::<Option<Vec<_>>>()?;
    if selectors.is_empty() {
        return None;
    }

    Some((base, LegadoJsoupIndexFilter { mode, selectors }))
}

fn parse_legado_jsoup_bracket_index_token(token: &str) -> Option<LegadoJsoupIndexSelector> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    if token.contains(':') {
        let parts = token.split(':').collect::<Vec<_>>();
        if parts.len() < 2 || parts.len() > 3 {
            return None;
        }

        return Some(LegadoJsoupIndexSelector::Range {
            start: parse_optional_legado_jsoup_index(parts[0].trim())?,
            end: parse_optional_legado_jsoup_index(parts[1].trim())?,
            step: if parts.len() == 3 {
                parts[2].trim().parse::<isize>().ok()?
            } else {
                1
            },
        });
    }

    Some(LegadoJsoupIndexSelector::Index(
        token.parse::<isize>().ok()?,
    ))
}

fn parse_optional_legado_jsoup_index(value: &str) -> Option<Option<isize>> {
    if value.is_empty() {
        Some(None)
    } else {
        value.parse::<isize>().ok().map(Some)
    }
}

fn parse_legado_jsoup_legacy_index_filter(
    selector: &str,
) -> Option<(&str, LegadoJsoupIndexFilter)> {
    for (index, value) in selector.char_indices().rev() {
        if value != '.' && value != '!' {
            continue;
        }

        let base = selector[..index].trim();
        let suffix = selector[index + value.len_utf8()..].trim();

        let selectors = parse_legado_jsoup_legacy_index_sequence(suffix)?;
        let mode = if value == '!' {
            LegadoJsoupIndexMode::Exclude
        } else {
            LegadoJsoupIndexMode::Select
        };
        return Some((base, LegadoJsoupIndexFilter { mode, selectors }));
    }

    None
}

fn parse_legado_jsoup_legacy_index_sequence(suffix: &str) -> Option<Vec<LegadoJsoupIndexSelector>> {
    let suffix = suffix.trim();
    if suffix.is_empty() {
        return None;
    }

    suffix
        .split(':')
        .map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            Some(LegadoJsoupIndexSelector::Index(part.parse::<isize>().ok()?))
        })
        .collect()
}

fn apply_legado_jsoup_index_filter<'a>(
    elements: Vec<ElementRef<'a>>,
    filter: Option<LegadoJsoupIndexFilter>,
) -> Vec<ElementRef<'a>> {
    let Some(filter) = filter else {
        return elements;
    };

    let indices = resolve_legado_jsoup_index_filter(&filter, elements.len());
    match filter.mode {
        LegadoJsoupIndexMode::Select => indices
            .into_iter()
            .filter_map(|index| elements.get(index).copied())
            .collect(),
        LegadoJsoupIndexMode::Exclude => elements
            .into_iter()
            .enumerate()
            .filter_map(|(index, element)| {
                if indices.contains(&index) {
                    None
                } else {
                    Some(element)
                }
            })
            .collect(),
    }
}

fn resolve_legado_jsoup_index_filter(filter: &LegadoJsoupIndexFilter, len: usize) -> Vec<usize> {
    let mut indices = Vec::new();
    if len == 0 {
        return indices;
    }

    for selector in &filter.selectors {
        match selector {
            LegadoJsoupIndexSelector::Index(index) => {
                if let Some(index) = resolve_legado_jsoup_index(*index, len) {
                    push_unique_index(&mut indices, index);
                }
            }
            LegadoJsoupIndexSelector::Range { start, end, step } => {
                resolve_legado_jsoup_range(*start, *end, *step, len, &mut indices);
            }
        }
    }

    indices
}

fn resolve_legado_jsoup_index(index: isize, len: usize) -> Option<usize> {
    let len = len as isize;
    if index >= 0 && index < len {
        Some(index as usize)
    } else if index < 0 && len >= -index {
        Some((index + len) as usize)
    } else {
        None
    }
}

fn resolve_legado_jsoup_range(
    start: Option<isize>,
    end: Option<isize>,
    step: isize,
    len: usize,
    indices: &mut Vec<usize>,
) {
    let len = len as isize;
    let mut start = start.unwrap_or(0);
    if start < 0 {
        start += len;
    }
    let mut end = end.unwrap_or(len - 1);
    if end < 0 {
        end += len;
    }

    if (start < 0 && end < 0) || (start >= len && end >= len) {
        return;
    }
    start = start.clamp(0, len - 1);
    end = end.clamp(0, len - 1);

    if start == end || step >= len {
        push_unique_index(indices, start as usize);
        return;
    }

    let step = if step > 0 {
        step
    } else if -step < len {
        step + len
    } else {
        1
    };
    let step = if step <= 0 { len } else { step };

    let mut current = start;
    if end > start {
        while current <= end {
            push_unique_index(indices, current as usize);
            current += step;
        }
    } else {
        loop {
            push_unique_index(indices, current as usize);
            let next = current - step;
            if next < end {
                break;
            }
            current = next;
        }
    }
}

fn push_unique_index(indices: &mut Vec<usize>, index: usize) {
    if !indices.contains(&index) {
        indices.push(index);
    }
}

fn legado_jsoup_default_candidates<'a>(
    contexts: &[LegadoCssContext<'a>],
) -> RuleResult<Vec<ElementRef<'a>>> {
    let universal = Selector::parse("*").map_err(|err| RuleError::CssSelectorSyntax {
        selector: "*".to_string(),
        message: format!("{err:?}"),
    })?;
    let mut elements = Vec::new();

    for context in contexts {
        match context {
            LegadoCssContext::Document(document) => elements.extend(document.select(&universal)),
            LegadoCssContext::Element(element) => {
                elements.push(*element);
                elements.extend(element.select(&universal));
            }
        }
    }

    Ok(elements)
}

fn is_legado_jsoup_children_selector(selector: &str) -> bool {
    selector.split('.').next() == Some("children")
}

fn legado_jsoup_direct_children<'a>(contexts: &[LegadoCssContext<'a>]) -> Vec<ElementRef<'a>> {
    let mut elements = Vec::new();

    for context in contexts {
        match context {
            LegadoCssContext::Document(document) => elements.push(document.root_element()),
            LegadoCssContext::Element(element) => elements.extend(element.child_elements()),
        }
    }

    elements
}

fn legado_jsoup_shorthand_matches(
    element: &ElementRef<'_>,
    shorthand: LegadoJsoupShorthandSelector<'_>,
) -> bool {
    match shorthand {
        LegadoJsoupShorthandSelector::Class(class_name) => element
            .value()
            .attr("class")
            .is_some_and(|value| value.split_whitespace().any(|item| item == class_name)),
        LegadoJsoupShorthandSelector::Tag(tag_name) => {
            tag_name == "*" || element.value().name() == tag_name
        }
        LegadoJsoupShorthandSelector::Id(id) => element.value().attr("id") == Some(id),
        LegadoJsoupShorthandSelector::OwnText(needle) => {
            let own_text = normalize_text(&element_own_text(element)).to_lowercase();
            own_text.contains(&needle.to_lowercase())
        }
    }
}

fn legado_css_context_elements<'a>(contexts: &[LegadoCssContext<'a>]) -> Vec<ElementRef<'a>> {
    contexts
        .iter()
        .map(|context| match context {
            LegadoCssContext::Document(document) => document.root_element(),
            LegadoCssContext::Element(element) => *element,
        })
        .collect()
}

fn extract_legado_css_values(
    elements: Vec<ElementRef<'_>>,
    extraction: &LegadoCssExtraction,
) -> Vec<String> {
    if matches!(
        extraction,
        LegadoCssExtraction::Html | LegadoCssExtraction::All
    ) {
        return extract_legado_css_outer_html(elements, extraction);
    }

    let mut output = Vec::new();

    for element in elements {
        match extraction {
            LegadoCssExtraction::Text => {
                let value = element_text(&element);
                if !value.is_empty() {
                    output.push(value);
                }
            }
            LegadoCssExtraction::TextNodes => {
                let value = element_legado_text_nodes(&element);
                if !value.is_empty() {
                    output.push(value);
                }
            }
            LegadoCssExtraction::OwnText => {
                let value = normalize_text(&element_own_text(&element));
                if !value.is_empty() {
                    output.push(value);
                }
            }
            LegadoCssExtraction::Attr(attr) => {
                if let Some(value) = element.value().attr(attr) {
                    if value.trim().is_empty() || output.iter().any(|item| item == value) {
                        continue;
                    }
                    output.push(value.to_string());
                }
            }
            LegadoCssExtraction::Html | LegadoCssExtraction::All => unreachable!(),
        }
    }

    output
}

fn extract_legado_css_outer_html(
    elements: Vec<ElementRef<'_>>,
    extraction: &LegadoCssExtraction,
) -> Vec<String> {
    if elements.is_empty() {
        return Vec::new();
    }

    let html = elements
        .into_iter()
        .map(|element| element.html())
        .collect::<Vec<_>>()
        .join("\n");
    let html = match extraction {
        LegadoCssExtraction::Html => clean_legado_html_extraction(&html),
        LegadoCssExtraction::All => html,
        _ => unreachable!(),
    };

    if html.is_empty() {
        Vec::new()
    } else {
        vec![html]
    }
}

fn clean_legado_html_extraction(html: &str) -> String {
    Regex::new(r"(?is)<script\b[^>]*>.*?</script>|<style\b[^>]*>.*?</style>")
        .expect("script/style cleanup regex must compile")
        .replace_all(html, "")
        .into_owned()
}

fn element_legado_text_nodes(element: &ElementRef<'_>) -> String {
    element
        .children()
        .filter_map(|child| {
            let text = child.value().as_text()?;
            let text = text.trim_matches(|value: char| value <= ' ');
            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn select_css_compat_elements<'a>(
    contexts: &[LegadoCssContext<'a>],
    selector_text: &str,
) -> RuleResult<Vec<ElementRef<'a>>> {
    let compat_selector = parse_css_compat_selector(selector_text).map_err(|message| {
        RuleError::CssSelectorSyntax {
            selector: selector_text.to_string(),
            message,
        }
    })?;
    let selector =
        Selector::parse(&compat_selector.selector).map_err(|err| RuleError::CssSelectorSyntax {
            selector: selector_text.to_string(),
            message: format!("{err:?}"),
        })?;
    let has_selector = compat_selector
        .has_selector_filter
        .as_ref()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: selector_text.to_string(),
                message: format!("{err:?}"),
            })
        })
        .transpose()?;
    let has_data_selector = compat_selector
        .has_data_filter
        .as_ref()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: selector_text.to_string(),
                message: format!("{err:?}"),
            })
        })
        .transpose()?;
    let has_text_selector = compat_selector
        .has_text_filter
        .as_ref()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: selector_text.to_string(),
                message: format!("{err:?}"),
            })
        })
        .transpose()?;
    let has_text_filter_regex = if let Some(filter) = &compat_selector.has_text_filter {
        if filter.text_filter.matcher == CssTextMatcher::Regex {
            Some(Regex::new(&filter.text_filter.value).map_err(|err| {
                RuleError::CssSelectorSyntax {
                    selector: selector_text.to_string(),
                    message: err.to_string(),
                }
            })?)
        } else {
            None
        }
    } else {
        None
    };
    let anchored_text_selectors = compat_selector
        .anchored_text_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: selector_text.to_string(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_text_filter_regexes = compat_selector
        .anchored_text_filters
        .iter()
        .map(|filter| {
            if filter.text_filter.matcher == CssTextMatcher::Regex {
                Regex::new(&filter.text_filter.value)
                    .map(Some)
                    .map_err(|err| RuleError::CssSelectorSyntax {
                        selector: selector_text.to_string(),
                        message: err.to_string(),
                    })
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_has_selectors = compat_selector
        .anchored_has_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: selector_text.to_string(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_data_selectors = compat_selector
        .anchored_data_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: selector_text.to_string(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_result_selectors = compat_selector
        .anchored_result_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: selector_text.to_string(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_parent_selectors = compat_selector
        .anchored_parent_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: selector_text.to_string(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let text_filter_regexes = compat_selector
        .text_filters
        .iter()
        .map(|filter| {
            if filter.text_filter.matcher == CssTextMatcher::Regex {
                Regex::new(&filter.text_filter.value)
                    .map(Some)
                    .map_err(|err| RuleError::CssSelectorSyntax {
                        selector: selector_text.to_string(),
                        message: err.to_string(),
                    })
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result_group_selectors = compat_selector
        .result_filter_groups
        .as_ref()
        .map(|groups| {
            groups
                .iter()
                .map(|group| {
                    Selector::parse(&group.selector).map_err(|err| RuleError::CssSelectorSyntax {
                        selector: selector_text.to_string(),
                        message: format!("{err:?}"),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let anchored_result_elements = compat_selector
        .anchored_result_filters
        .iter()
        .zip(anchored_result_selectors.iter())
        .map(|(anchored_result_filter, anchored_result_selector)| {
            let mut anchors = Vec::new();
            for context in contexts {
                match context {
                    LegadoCssContext::Document(document) => {
                        anchors.extend(document.select(anchored_result_selector));
                    }
                    LegadoCssContext::Element(element) => {
                        anchors.extend(element.select(anchored_result_selector));
                    }
                }
            }
            apply_css_result_filters(anchors, &anchored_result_filter.filters)
        })
        .collect::<Vec<_>>();

    let collect_matching_elements = |selector: &Selector| {
        let mut elements = Vec::new();
        for context in contexts {
            let selected: Vec<ElementRef<'a>> = match context {
                LegadoCssContext::Document(document) => document.select(selector).collect(),
                LegadoCssContext::Element(element) => element.select(selector).collect(),
            };

            for element in selected {
                if let Some(parent_filter) = compat_selector.parent_filter {
                    if !css_parent_filter_matches(&element, parent_filter) {
                        continue;
                    }
                }
                if let Some(has_selector_filter) = &compat_selector.has_selector_filter {
                    let Some(has_selector) = has_selector.as_ref() else {
                        continue;
                    };
                    let matches = !css_has_filter_candidates(
                        &element,
                        has_selector,
                        has_selector_filter.direct_child,
                        &has_selector_filter.result_filters,
                        has_selector_filter.parent_filter,
                        has_selector_filter.nested_filter.as_ref(),
                    )
                    .is_empty();
                    match has_selector_filter.mode {
                        CssTextFilterMode::Contains if !matches => continue,
                        CssTextFilterMode::NotContains if matches => continue,
                        _ => {}
                    }
                }
                if let Some(has_data_filter) = &compat_selector.has_data_filter {
                    let Some(has_data_selector) = has_data_selector.as_ref() else {
                        continue;
                    };
                    let has_matching_data = css_has_filter_candidates(
                        &element,
                        has_data_selector,
                        has_data_filter.direct_child,
                        &has_data_filter.result_filters,
                        has_data_filter.parent_filter,
                        None,
                    )
                    .into_iter()
                    .any(|descendant| {
                        css_element_data_filter_matches(&descendant, &has_data_filter.data_filter)
                    });
                    match has_data_filter.mode {
                        CssTextFilterMode::Contains if !has_matching_data => continue,
                        CssTextFilterMode::NotContains if has_matching_data => continue,
                        _ => {}
                    }
                }
                if let Some(has_text_filter) = &compat_selector.has_text_filter {
                    let Some(has_text_selector) = has_text_selector.as_ref() else {
                        continue;
                    };
                    let has_matching_text = css_has_filter_candidates(
                        &element,
                        has_text_selector,
                        has_text_filter.direct_child,
                        &has_text_filter.result_filters,
                        has_text_filter.parent_filter,
                        None,
                    )
                    .into_iter()
                    .any(|descendant| {
                        css_element_text_filter_mode_matches(
                            &descendant,
                            &has_text_filter.text_filter,
                            has_text_filter.text_filter_mode,
                            has_text_filter_regex.as_ref(),
                        )
                    });
                    match has_text_filter.mode {
                        CssTextFilterMode::Contains if !has_matching_text => continue,
                        CssTextFilterMode::NotContains if has_matching_text => continue,
                        _ => {}
                    }
                }
                if !compat_selector.anchored_text_filters.is_empty()
                    && !compat_selector
                        .anchored_text_filters
                        .iter()
                        .zip(anchored_text_selectors.iter())
                        .zip(anchored_text_filter_regexes.iter())
                        .all(|((anchored_text_filter, anchored_text_selector), regex)| {
                            element
                                .ancestors()
                                .filter_map(ElementRef::wrap)
                                .any(|ancestor| {
                                    anchored_text_selector.matches(&ancestor)
                                        && match anchored_text_filter.mode {
                                            CssTextFilterMode::Contains => {
                                                css_element_text_filter_matches(
                                                    &ancestor,
                                                    &anchored_text_filter.text_filter,
                                                    regex.as_ref(),
                                                )
                                            }
                                            CssTextFilterMode::NotContains => {
                                                !css_element_text_filter_matches(
                                                    &ancestor,
                                                    &anchored_text_filter.text_filter,
                                                    regex.as_ref(),
                                                )
                                            }
                                        }
                                })
                        })
                {
                    continue;
                }
                if !compat_selector.anchored_has_filters.is_empty()
                    && !compat_selector
                        .anchored_has_filters
                        .iter()
                        .zip(anchored_has_selectors.iter())
                        .all(|(anchored_has_filter, anchored_has_selector)| {
                            element
                                .ancestors()
                                .filter_map(ElementRef::wrap)
                                .any(|ancestor| {
                                    anchored_has_selector.matches(&ancestor)
                                        && css_nested_has_filter_matches(
                                            &ancestor,
                                            &anchored_has_filter.filter,
                                        )
                                })
                        })
                {
                    continue;
                }
                if !compat_selector.anchored_data_filters.is_empty()
                    && !compat_selector
                        .anchored_data_filters
                        .iter()
                        .zip(anchored_data_selectors.iter())
                        .all(|(anchored_data_filter, anchored_data_selector)| {
                            element
                                .ancestors()
                                .filter_map(ElementRef::wrap)
                                .any(|ancestor| {
                                    anchored_data_selector.matches(&ancestor)
                                        && css_element_data_filter_matches(
                                            &ancestor,
                                            &anchored_data_filter.data_filter,
                                        )
                                })
                        })
                {
                    continue;
                }
                if !anchored_result_elements.is_empty()
                    && !anchored_result_elements
                        .iter()
                        .all(|anchors| css_element_has_anchored_result_match(&element, anchors))
                {
                    continue;
                }
                if !compat_selector.anchored_parent_filters.is_empty()
                    && !compat_selector
                        .anchored_parent_filters
                        .iter()
                        .zip(anchored_parent_selectors.iter())
                        .all(|(anchored_parent_filter, anchored_parent_selector)| {
                            element
                                .ancestors()
                                .filter_map(ElementRef::wrap)
                                .any(|ancestor| {
                                    anchored_parent_selector.matches(&ancestor)
                                        && css_parent_filter_matches(
                                            &ancestor,
                                            anchored_parent_filter.parent_filter,
                                        )
                                })
                        })
                {
                    continue;
                }
                if !compat_selector
                    .data_filters
                    .iter()
                    .all(|data_filter| css_element_data_filter_matches(&element, data_filter))
                {
                    continue;
                }

                if !compat_selector.text_filters.is_empty()
                    && !compat_selector
                        .text_filters
                        .iter()
                        .zip(text_filter_regexes.iter())
                        .all(|(filter, regex)| {
                            let haystack =
                                css_element_text_filter_haystack(&element, &filter.text_filter);
                            let matches = css_text_filter_matches(
                                &haystack,
                                &filter.text_filter,
                                regex.as_ref(),
                            );
                            match filter.mode {
                                CssTextFilterMode::Contains => matches,
                                CssTextFilterMode::NotContains => !matches,
                            }
                        })
                {
                    continue;
                }

                elements.push(element);
            }
        }
        elements
    };

    if let Some(groups) = &compat_selector.result_filter_groups {
        let Some(group_selectors) = result_group_selectors.as_ref() else {
            return Err(RuleError::CssSelectorSyntax {
                selector: selector_text.to_string(),
                message: "CSS result filter groups were not compiled".to_string(),
            });
        };
        let mut elements = Vec::new();
        for (group, selector) in groups.iter().zip(group_selectors) {
            let group_elements = collect_matching_elements(selector);
            elements.extend(apply_css_result_filters(group_elements, &group.filters));
        }
        Ok(elements)
    } else {
        Ok(apply_css_result_filters(
            collect_matching_elements(&selector),
            &compat_selector.result_filters,
        ))
    }
}

fn extract_css_attr_value(element: &ElementRef<'_>, attr: &str) -> Option<String> {
    if attr == "html" {
        let value = element.inner_html();
        if !value.is_empty() {
            return Some(value);
        }
    } else if attr == "textNodes" {
        let value = element_text(element);
        if !value.is_empty() {
            return Some(value);
        }
    } else if attr == "ownText" {
        let value = normalize_text(&element_own_text(element));
        if !value.is_empty() {
            return Some(value);
        }
    } else if let Some(value) = element.value().attr(attr) {
        return Some(value.to_string());
    }

    None
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
    let has_selector = compat_selector
        .has_selector_filter
        .as_ref()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: format!("{err:?}"),
            })
        })
        .transpose()?;
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
    let anchored_text_selectors = compat_selector
        .anchored_text_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_text_filter_regexes = compat_selector
        .anchored_text_filters
        .iter()
        .map(|filter| {
            if filter.text_filter.matcher == CssTextMatcher::Regex {
                Regex::new(&filter.text_filter.value)
                    .map(Some)
                    .map_err(|err| RuleError::CssSelectorSyntax {
                        selector: rule.selector.clone(),
                        message: err.to_string(),
                    })
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_has_selectors = compat_selector
        .anchored_has_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_data_selectors = compat_selector
        .anchored_data_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_result_selectors = compat_selector
        .anchored_result_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchored_parent_selectors = compat_selector
        .anchored_parent_filters
        .iter()
        .map(|filter| {
            Selector::parse(&filter.selector).map_err(|err| RuleError::CssSelectorSyntax {
                selector: rule.selector.clone(),
                message: format!("{err:?}"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let text_filter_regexes = compat_selector
        .text_filters
        .iter()
        .map(|filter| {
            if filter.text_filter.matcher == CssTextMatcher::Regex {
                Regex::new(&filter.text_filter.value)
                    .map(Some)
                    .map_err(|err| RuleError::CssSelectorSyntax {
                        selector: rule.selector.clone(),
                        message: err.to_string(),
                    })
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
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
    let anchored_result_elements = compat_selector
        .anchored_result_filters
        .iter()
        .zip(anchored_result_selectors.iter())
        .map(|(anchored_result_filter, anchored_result_selector)| {
            apply_css_result_filters(
                document.select(anchored_result_selector).collect(),
                &anchored_result_filter.filters,
            )
        })
        .collect::<Vec<_>>();

    let collect_matching_elements = |selector: &Selector| {
        let mut elements = Vec::new();
        for element in document.select(selector) {
            if let Some(parent_filter) = compat_selector.parent_filter {
                if !css_parent_filter_matches(&element, parent_filter) {
                    continue;
                }
            }
            if let Some(has_selector_filter) = &compat_selector.has_selector_filter {
                let Some(has_selector) = has_selector.as_ref() else {
                    continue;
                };
                let matches = !css_has_filter_candidates(
                    &element,
                    has_selector,
                    has_selector_filter.direct_child,
                    &has_selector_filter.result_filters,
                    has_selector_filter.parent_filter,
                    has_selector_filter.nested_filter.as_ref(),
                )
                .is_empty();
                match has_selector_filter.mode {
                    CssTextFilterMode::Contains if !matches => continue,
                    CssTextFilterMode::NotContains if matches => continue,
                    _ => {}
                }
            }
            if let Some(has_data_filter) = &compat_selector.has_data_filter {
                let Some(has_data_selector) = has_data_selector.as_ref() else {
                    continue;
                };
                let has_matching_data = css_has_filter_candidates(
                    &element,
                    has_data_selector,
                    has_data_filter.direct_child,
                    &has_data_filter.result_filters,
                    has_data_filter.parent_filter,
                    None,
                )
                .into_iter()
                .any(|descendant| {
                    css_element_data_filter_matches(&descendant, &has_data_filter.data_filter)
                });
                match has_data_filter.mode {
                    CssTextFilterMode::Contains if !has_matching_data => continue,
                    CssTextFilterMode::NotContains if has_matching_data => continue,
                    _ => {}
                }
            }
            if let Some(has_text_filter) = &compat_selector.has_text_filter {
                let Some(has_text_selector) = has_text_selector.as_ref() else {
                    continue;
                };
                let has_matching_text = css_has_filter_candidates(
                    &element,
                    has_text_selector,
                    has_text_filter.direct_child,
                    &has_text_filter.result_filters,
                    has_text_filter.parent_filter,
                    None,
                )
                .into_iter()
                .any(|descendant| {
                    css_element_text_filter_mode_matches(
                        &descendant,
                        &has_text_filter.text_filter,
                        has_text_filter.text_filter_mode,
                        has_text_filter_regex.as_ref(),
                    )
                });
                match has_text_filter.mode {
                    CssTextFilterMode::Contains if !has_matching_text => continue,
                    CssTextFilterMode::NotContains if has_matching_text => continue,
                    _ => {}
                }
            }
            if !compat_selector.anchored_text_filters.is_empty()
                && !compat_selector
                    .anchored_text_filters
                    .iter()
                    .zip(anchored_text_selectors.iter())
                    .zip(anchored_text_filter_regexes.iter())
                    .all(|((anchored_text_filter, anchored_text_selector), regex)| {
                        element
                            .ancestors()
                            .filter_map(ElementRef::wrap)
                            .any(|ancestor| {
                                anchored_text_selector.matches(&ancestor)
                                    && match anchored_text_filter.mode {
                                        CssTextFilterMode::Contains => {
                                            css_element_text_filter_matches(
                                                &ancestor,
                                                &anchored_text_filter.text_filter,
                                                regex.as_ref(),
                                            )
                                        }
                                        CssTextFilterMode::NotContains => {
                                            !css_element_text_filter_matches(
                                                &ancestor,
                                                &anchored_text_filter.text_filter,
                                                regex.as_ref(),
                                            )
                                        }
                                    }
                            })
                    })
            {
                continue;
            }
            if !compat_selector.anchored_has_filters.is_empty()
                && !compat_selector
                    .anchored_has_filters
                    .iter()
                    .zip(anchored_has_selectors.iter())
                    .all(|(anchored_has_filter, anchored_has_selector)| {
                        element
                            .ancestors()
                            .filter_map(ElementRef::wrap)
                            .any(|ancestor| {
                                anchored_has_selector.matches(&ancestor)
                                    && css_nested_has_filter_matches(
                                        &ancestor,
                                        &anchored_has_filter.filter,
                                    )
                            })
                    })
            {
                continue;
            }
            if !compat_selector.anchored_data_filters.is_empty()
                && !compat_selector
                    .anchored_data_filters
                    .iter()
                    .zip(anchored_data_selectors.iter())
                    .all(|(anchored_data_filter, anchored_data_selector)| {
                        element
                            .ancestors()
                            .filter_map(ElementRef::wrap)
                            .any(|ancestor| {
                                anchored_data_selector.matches(&ancestor)
                                    && css_element_data_filter_matches(
                                        &ancestor,
                                        &anchored_data_filter.data_filter,
                                    )
                            })
                    })
            {
                continue;
            }
            if !anchored_result_elements.is_empty()
                && !anchored_result_elements
                    .iter()
                    .all(|anchors| css_element_has_anchored_result_match(&element, anchors))
            {
                continue;
            }
            if !compat_selector.anchored_parent_filters.is_empty()
                && !compat_selector
                    .anchored_parent_filters
                    .iter()
                    .zip(anchored_parent_selectors.iter())
                    .all(|(anchored_parent_filter, anchored_parent_selector)| {
                        element
                            .ancestors()
                            .filter_map(ElementRef::wrap)
                            .any(|ancestor| {
                                anchored_parent_selector.matches(&ancestor)
                                    && css_parent_filter_matches(
                                        &ancestor,
                                        anchored_parent_filter.parent_filter,
                                    )
                            })
                    })
            {
                continue;
            }
            if !compat_selector
                .data_filters
                .iter()
                .all(|data_filter| css_element_data_filter_matches(&element, data_filter))
            {
                continue;
            }

            if !compat_selector.text_filters.is_empty()
                && !compat_selector
                    .text_filters
                    .iter()
                    .zip(text_filter_regexes.iter())
                    .all(|(filter, regex)| {
                        let haystack =
                            css_element_text_filter_haystack(&element, &filter.text_filter);
                        let matches =
                            css_text_filter_matches(&haystack, &filter.text_filter, regex.as_ref());
                        match filter.mode {
                            CssTextFilterMode::Contains => matches,
                            CssTextFilterMode::NotContains => !matches,
                        }
                    })
            {
                continue;
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
                if let Some(value) = extract_css_attr_value(&element, attr) {
                    output.push(value);
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

fn css_element_has_anchored_result_match(
    element: &ElementRef<'_>,
    anchors: &[ElementRef<'_>],
) -> bool {
    element
        .ancestors()
        .filter_map(ElementRef::wrap)
        .any(|ancestor| anchors.iter().any(|anchor| *anchor == ancestor))
}

fn css_parent_filter_matches(element: &ElementRef<'_>, parent_filter: CssParentFilter) -> bool {
    let has_child_nodes = element_has_child_nodes(element);
    match parent_filter {
        CssParentFilter::HasChildren => has_child_nodes,
        CssParentFilter::Empty => !has_child_nodes,
    }
}

fn css_has_filter_candidates<'a>(
    element: &ElementRef<'a>,
    selector: &Selector,
    direct_child: bool,
    filters: &[CssResultFilter],
    parent_filter: Option<CssParentFilter>,
    nested_filter: Option<&CssNestedHasFilter>,
) -> Vec<ElementRef<'a>> {
    let mut elements = if direct_child {
        element
            .child_elements()
            .filter(|child| selector.matches(child))
            .collect::<Vec<_>>()
    } else {
        element.select(selector).collect::<Vec<_>>()
    };

    if let Some(parent_filter) = parent_filter {
        elements.retain(|element| css_parent_filter_matches(element, parent_filter));
    }

    if let Some(nested_filter) = nested_filter {
        elements.retain(|element| css_nested_has_filter_matches(element, nested_filter));
    }

    apply_css_result_filters(elements, filters)
}

fn css_has_selector_filter_matches(
    element: &ElementRef<'_>,
    filter: &CssHasSelectorFilter,
) -> bool {
    let Ok(selector) = Selector::parse(&filter.selector) else {
        return false;
    };
    let matches = !css_has_filter_candidates(
        element,
        &selector,
        filter.direct_child,
        &filter.result_filters,
        filter.parent_filter,
        filter.nested_filter.as_ref(),
    )
    .is_empty();

    match filter.mode {
        CssTextFilterMode::Contains => matches,
        CssTextFilterMode::NotContains => !matches,
    }
}

fn css_nested_has_filter_matches(element: &ElementRef<'_>, filter: &CssNestedHasFilter) -> bool {
    match filter {
        CssNestedHasFilter::Selector(filter) => css_has_selector_filter_matches(element, filter),
        CssNestedHasFilter::Data(filter) => css_has_data_filter_matches(element, filter),
        CssNestedHasFilter::Text(filter) => css_has_text_filter_matches(element, filter),
    }
}

fn css_has_data_filter_matches(element: &ElementRef<'_>, filter: &CssHasDataFilter) -> bool {
    let Ok(selector) = Selector::parse(&filter.selector) else {
        return false;
    };
    let has_matching_data = css_has_filter_candidates(
        element,
        &selector,
        filter.direct_child,
        &filter.result_filters,
        filter.parent_filter,
        None,
    )
    .into_iter()
    .any(|candidate| css_element_data_filter_matches(&candidate, &filter.data_filter));

    match filter.mode {
        CssTextFilterMode::Contains => has_matching_data,
        CssTextFilterMode::NotContains => !has_matching_data,
    }
}

fn css_has_text_filter_matches(element: &ElementRef<'_>, filter: &CssHasTextFilter) -> bool {
    let Ok(selector) = Selector::parse(&filter.selector) else {
        return false;
    };
    let regex = if filter.text_filter.matcher == CssTextMatcher::Regex {
        match Regex::new(&filter.text_filter.value) {
            Ok(regex) => Some(regex),
            Err(_) => return false,
        }
    } else {
        None
    };
    let has_matching_text = css_has_filter_candidates(
        element,
        &selector,
        filter.direct_child,
        &filter.result_filters,
        filter.parent_filter,
        None,
    )
    .into_iter()
    .any(|candidate| {
        css_element_text_filter_mode_matches(
            &candidate,
            &filter.text_filter,
            filter.text_filter_mode,
            regex.as_ref(),
        )
    });

    match filter.mode {
        CssTextFilterMode::Contains => has_matching_text,
        CssTextFilterMode::NotContains => !has_matching_text,
    }
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

fn css_element_text_filter_mode_matches(
    element: &ElementRef<'_>,
    filter: &CssTextFilter,
    mode: CssTextFilterMode,
    regex: Option<&Regex>,
) -> bool {
    let matches = css_element_text_filter_matches(element, filter, regex);
    match mode {
        CssTextFilterMode::Contains => matches,
        CssTextFilterMode::NotContains => !matches,
    }
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

fn css_element_data_filter_matches(element: &ElementRef<'_>, filter: &CssDataFilter) -> bool {
    let matches = css_contains_data_matches(element, &filter.value);
    match filter.mode {
        CssDataFilterMode::Contains => matches,
        CssDataFilterMode::NotContains => !matches,
    }
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
    text_filters: Vec<CssTextFilterPredicate>,
    result_filters: Vec<CssResultFilter>,
    result_filter_groups: Option<Vec<CssResultFilterGroup>>,
    data_filters: Vec<CssDataFilter>,
    has_selector_filter: Option<CssHasSelectorFilter>,
    has_data_filter: Option<CssHasDataFilter>,
    has_text_filter: Option<CssHasTextFilter>,
    anchored_text_filters: Vec<CssAnchoredTextFilter>,
    anchored_has_filters: Vec<CssAnchoredHasFilter>,
    anchored_data_filters: Vec<CssAnchoredDataFilter>,
    anchored_result_filters: Vec<CssAnchoredResultFilter>,
    anchored_parent_filters: Vec<CssAnchoredParentFilter>,
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
struct CssAnchoredHasFilter {
    selector: String,
    filter: CssNestedHasFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssAnchoredDataFilter {
    selector: String,
    data_filter: CssDataFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssAnchoredResultFilter {
    selector: String,
    filters: Vec<CssResultFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssAnchoredParentFilter {
    selector: String,
    parent_filter: CssParentFilter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssHasSelectorFilter {
    selector: String,
    direct_child: bool,
    result_filters: Vec<CssResultFilter>,
    parent_filter: Option<CssParentFilter>,
    nested_filter: Option<CssNestedHasFilter>,
    mode: CssTextFilterMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CssNestedHasFilter {
    Selector(Box<CssHasSelectorFilter>),
    Data(CssHasDataFilter),
    Text(CssHasTextFilter),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssHasDataFilter {
    selector: String,
    direct_child: bool,
    result_filters: Vec<CssResultFilter>,
    parent_filter: Option<CssParentFilter>,
    data_filter: CssDataFilter,
    mode: CssTextFilterMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssHasTextFilter {
    selector: String,
    direct_child: bool,
    result_filters: Vec<CssResultFilter>,
    parent_filter: Option<CssParentFilter>,
    text_filter: CssTextFilter,
    text_filter_mode: CssTextFilterMode,
    mode: CssTextFilterMode,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssTextFilterPredicate {
    text_filter: CssTextFilter,
    mode: CssTextFilterMode,
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

    let mut anchored_result_filters = Vec::new();
    if split_css_selector_groups(&selector).len() == 1 {
        while let Some((rewritten, filter)) = extract_css_anchored_result_filter(&selector)? {
            selector = rewritten;
            anchored_result_filters.push(filter);
        }
    }
    let mut anchored_parent_filters = Vec::new();
    if split_css_selector_groups(&selector).len() == 1 {
        while let Some((rewritten, filter)) = extract_css_anchored_parent_filter(&selector)? {
            selector = rewritten;
            anchored_parent_filters.push(filter);
        }
    }

    let (rewritten_selector, result_filters, result_filter_groups) =
        extract_css_result_filter_groups(&selector)?;
    selector = rewritten_selector;

    let mut anchored_text_filters = Vec::new();
    while let Some((rewritten, filter)) = extract_css_anchored_text_filter(&selector)? {
        selector = rewritten;
        anchored_text_filters.push(filter);
    }
    let mut anchored_has_filters = Vec::new();
    while let Some((rewritten, filter)) = extract_css_anchored_has_filter(&selector)? {
        selector = rewritten;
        anchored_has_filters.push(filter);
    }
    let mut anchored_data_filters = Vec::new();
    while let Some((rewritten, filter)) = extract_css_anchored_data_filter(&selector)? {
        selector = rewritten;
        anchored_data_filters.push(filter);
    }

    let mut has_selector_filter = None;
    let mut has_data_filter = None;
    let mut has_text_filter = None;
    if let Some(suffix) = find_css_not_has_filter_suffix(&selector) {
        let close_index = selector.len() - 2;
        let base = selector[..suffix.open_index].trim();
        let inner = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
        if inner.is_empty() {
            return Err(format!("CSS {} requires selector", suffix.name));
        }
        if find_css_not_data_filter_suffix(inner).is_some()
            || find_css_data_filter_suffix(inner).is_some()
        {
            has_data_filter = Some(parse_css_has_data_filter(
                inner,
                CssTextFilterMode::NotContains,
            )?);
        } else if find_css_not_text_filter_suffix(inner).is_some()
            || find_css_text_filter_suffix(inner).is_some()
        {
            has_text_filter = Some(parse_css_has_text_filter(
                inner,
                CssTextFilterMode::NotContains,
            )?);
        } else {
            has_selector_filter = Some(parse_css_has_selector_filter(
                inner,
                CssTextFilterMode::NotContains,
            )?);
        }
        selector = if base.is_empty() {
            "*".to_string()
        } else {
            base.to_string()
        };
    } else if let Some(suffix) = find_css_has_filter_suffix(&selector) {
        let close_index = selector.len() - 1;
        let base = selector[..suffix.open_index].trim();
        let inner = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
        if inner.is_empty() {
            return Err(format!("CSS {} requires selector", suffix.name));
        }
        if find_css_not_data_filter_suffix(inner).is_some()
            || find_css_data_filter_suffix(inner).is_some()
        {
            has_data_filter = Some(parse_css_has_data_filter(
                inner,
                CssTextFilterMode::Contains,
            )?);
        } else if find_css_not_text_filter_suffix(inner).is_some()
            || find_css_text_filter_suffix(inner).is_some()
        {
            has_text_filter = Some(parse_css_has_text_filter(
                inner,
                CssTextFilterMode::Contains,
            )?);
        } else {
            has_selector_filter = Some(parse_css_has_selector_filter(
                inner,
                CssTextFilterMode::Contains,
            )?);
        }
        selector = if base.is_empty() {
            "*".to_string()
        } else {
            base.to_string()
        };
    }

    let mut data_filters = Vec::new();
    loop {
        let data_filter = if let Some(suffix) = find_css_not_data_filter_suffix(&selector) {
            Some((suffix, selector.len() - 2, CssDataFilterMode::NotContains))
        } else {
            find_css_data_filter_suffix(&selector)
                .map(|suffix| (suffix, selector.len() - 1, CssDataFilterMode::Contains))
        };

        let Some((suffix, close_index, mode)) = data_filter else {
            break;
        };

        let base = selector[..suffix.open_index].trim();
        let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
        if argument.is_empty() {
            return Err(format!("CSS {} requires text", suffix.name));
        }
        data_filters.push(CssDataFilter {
            value: parse_css_text_filter_argument(argument, suffix.name)?,
            mode,
        });
        selector = if base.is_empty() {
            "*".to_string()
        } else {
            base.to_string()
        };
    }

    let mut text_filters = Vec::new();
    loop {
        let text_filter = if let Some(suffix) = find_css_not_text_filter_suffix(&selector) {
            Some((suffix, selector.len() - 2, CssTextFilterMode::NotContains))
        } else {
            find_css_text_filter_suffix(&selector)
                .map(|suffix| (suffix, selector.len() - 1, CssTextFilterMode::Contains))
        };

        let Some((suffix, close_index, mode)) = text_filter else {
            break;
        };

        let base = selector[..suffix.open_index].trim();
        let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
        if argument.is_empty() {
            return Err(format!("CSS {} requires text", suffix.name));
        }

        text_filters.push(CssTextFilterPredicate {
            text_filter: CssTextFilter {
                value: parse_css_text_filter_argument(argument, suffix.name)?,
                scope: suffix.scope,
                matcher: suffix.matcher,
            },
            mode,
        });
        selector = if base.is_empty() {
            "*".to_string()
        } else {
            base.to_string()
        };
    }

    let selector = if selector.is_empty() {
        "*".to_string()
    } else {
        selector
    };

    Ok(CssCompatSelector {
        selector: normalize_css_unquoted_attribute_values(&selector),
        text_filters,
        result_filters,
        result_filter_groups,
        data_filters,
        has_selector_filter,
        has_data_filter,
        has_text_filter,
        anchored_text_filters,
        anchored_has_filters,
        anchored_data_filters,
        anchored_result_filters,
        anchored_parent_filters,
        parent_filter,
    })
}

fn normalize_css_unquoted_attribute_values(selector: &str) -> String {
    let mut output = String::with_capacity(selector.len());
    let mut index = 0usize;

    while index < selector.len() {
        let value = selector[index..]
            .chars()
            .next()
            .expect("index is inside selector bounds");

        if value != '[' {
            output.push(value);
            index += value.len_utf8();
            continue;
        }

        let Some(end) = find_css_attribute_selector_end(selector, index) else {
            output.push_str(&selector[index..]);
            break;
        };
        output.push('[');
        output.push_str(&normalize_css_attribute_selector_content(
            &selector[index + '['.len_utf8()..end],
        ));
        output.push(']');
        index = end + ']'.len_utf8();
    }

    output
}

fn find_css_attribute_selector_end(selector: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut index = start + '['.len_utf8();

    while index < selector.len() {
        let value = selector[index..].chars().next()?;

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

        if value == ']' {
            return Some(index);
        }

        index += value.len_utf8();
    }

    None
}

fn normalize_css_attribute_selector_content(content: &str) -> String {
    let Some(equal_index) = content.find('=') else {
        return content.to_string();
    };

    let mut value_start = equal_index + '='.len_utf8();
    while value_start < content.len() {
        let value = content[value_start..]
            .chars()
            .next()
            .expect("index is inside attribute selector bounds");
        if !value.is_whitespace() {
            break;
        }
        value_start += value.len_utf8();
    }

    if value_start >= content.len() {
        return content.to_string();
    }

    let first = content[value_start..]
        .chars()
        .next()
        .expect("index is inside attribute selector bounds");
    if first == '"' || first == '\'' {
        return content.to_string();
    }

    let mut value_end = value_start;
    while value_end < content.len() {
        let value = content[value_end..]
            .chars()
            .next()
            .expect("index is inside attribute selector bounds");
        if value.is_whitespace() {
            break;
        }
        value_end += value.len_utf8();
    }

    if value_end == value_start {
        return content.to_string();
    }

    let mut output = String::with_capacity(content.len() + 2);
    output.push_str(&content[..value_start]);
    output.push('"');
    for value in content[value_start..value_end].chars() {
        if value == '"' || value == '\\' {
            output.push('\\');
        }
        output.push(value);
    }
    output.push('"');
    output.push_str(&content[value_end..]);
    output
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

fn extract_css_anchored_result_filter(
    selector: &str,
) -> Result<Option<(String, CssAnchoredResultFilter)>, String> {
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

        if value == ':' && bracket_depth == 0 && paren_depth == 0 {
            let mut filters = Vec::new();
            let mut cursor = index;
            while let Some((filter, consumed)) = parse_css_result_filter_at(selector, cursor)? {
                filters.push(filter);
                cursor += consumed;
            }

            if !filters.is_empty() {
                let tail = &selector[cursor..];
                if tail.trim().is_empty() {
                    return Ok(None);
                }

                let base = selector[..index].trim_end();
                if base.is_empty() {
                    return Ok(None);
                }

                return Ok(Some((
                    format!("{base}{tail}"),
                    CssAnchoredResultFilter {
                        selector: base.to_string(),
                        filters,
                    },
                )));
            }
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }
        index += value.len_utf8();
    }

    Ok(None)
}

fn extract_css_anchored_parent_filter(
    selector: &str,
) -> Result<Option<(String, CssAnchoredParentFilter)>, String> {
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
            if selector[index..].starts_with(":not(:parent)") {
                let tail_index = index + ":not(:parent)".len();
                let tail = &selector[tail_index..];
                if tail.trim().is_empty() {
                    return Ok(None);
                }
                let base = selector[..index].trim_end();
                if base.is_empty() {
                    return Ok(None);
                }
                return Ok(Some((
                    format!("{base}{tail}"),
                    CssAnchoredParentFilter {
                        selector: base.to_string(),
                        parent_filter: CssParentFilter::Empty,
                    },
                )));
            }

            if css_result_keyword_matches(&selector[index..], ":parent") {
                let tail_index = index + ":parent".len();
                let tail = &selector[tail_index..];
                if tail.trim().is_empty() {
                    return Ok(None);
                }
                let base = selector[..index].trim_end();
                if base.is_empty() {
                    return Ok(None);
                }
                return Ok(Some((
                    format!("{base}{tail}"),
                    CssAnchoredParentFilter {
                        selector: base.to_string(),
                        parent_filter: CssParentFilter::HasChildren,
                    },
                )));
            }
        }

        match value {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }
        index += value.len_utf8();
    }

    Ok(None)
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

fn parse_css_result_filter_at(
    selector: &str,
    index: usize,
) -> Result<Option<(CssResultFilter, usize)>, String> {
    if let Some((filter, consumed)) = css_result_filter_keyword(&selector[index..]) {
        return Ok(Some((filter, consumed)));
    }

    let Some((kind, prefix, name)) = css_result_filter_prefix(&selector[index..]) else {
        return Ok(None);
    };
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

    Ok(Some((
        CssResultFilter {
            kind,
            index: index_value,
        },
        close_index + 1 - index,
    )))
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
    if let Some(suffix) = find_first_css_not_text_filter_function(selector) {
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

    let Some(suffix) = find_first_css_text_filter_function(selector) else {
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

fn extract_css_anchored_has_filter(
    selector: &str,
) -> Result<Option<(String, CssAnchoredHasFilter)>, String> {
    let Some((suffix, mode)) = find_first_css_has_filter_for_anchor(selector) else {
        return Ok(None);
    };

    let argument_start = suffix.open_index + suffix.prefix_len;
    let close_index = matching_css_function_close(selector, argument_start)
        .ok_or_else(|| format!("unterminated CSS {} argument in `{selector}`", suffix.name))?;
    let tail_index = if mode == CssTextFilterMode::NotContains {
        let outer_close_index = close_index + 1;
        if selector.as_bytes().get(outer_close_index) != Some(&b')') {
            return Err(format!(
                "unterminated CSS {} argument in `{selector}`",
                suffix.name
            ));
        }
        outer_close_index + 1
    } else {
        close_index + 1
    };
    let tail = &selector[tail_index..];
    if tail.trim().is_empty() {
        return Ok(None);
    }
    if !css_selector_tail_has_top_level_combinator(tail) {
        return Ok(None);
    }

    let base = selector[..suffix.open_index].trim_end();
    if base.is_empty() {
        return Ok(None);
    }

    let inner = selector[argument_start..close_index].trim();
    if inner.is_empty() {
        return Err(format!("CSS {} requires selector", suffix.name));
    }

    Ok(Some((
        format!("{base}{tail}"),
        CssAnchoredHasFilter {
            selector: base.to_string(),
            filter: parse_css_nested_has_filter(inner, mode)?,
        },
    )))
}

fn extract_css_anchored_data_filter(
    selector: &str,
) -> Result<Option<(String, CssAnchoredDataFilter)>, String> {
    let Some((suffix, mode)) = find_first_css_data_filter_for_anchor(selector) else {
        return Ok(None);
    };

    let argument_start = suffix.open_index + suffix.prefix_len;
    let close_index = matching_css_function_close(selector, argument_start)
        .ok_or_else(|| format!("unterminated CSS {} argument in `{selector}`", suffix.name))?;
    let tail_index = if mode == CssDataFilterMode::NotContains {
        let outer_close_index = close_index + 1;
        if selector.as_bytes().get(outer_close_index) != Some(&b')') {
            return Err(format!(
                "unterminated CSS {} argument in `{selector}`",
                suffix.name
            ));
        }
        outer_close_index + 1
    } else {
        close_index + 1
    };
    let tail = &selector[tail_index..];
    if tail.trim().is_empty() {
        return Ok(None);
    }
    if !css_selector_tail_has_top_level_combinator(tail) {
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
        CssAnchoredDataFilter {
            selector: base.to_string(),
            data_filter: CssDataFilter {
                value: parse_css_text_filter_argument(argument, suffix.name)?,
                mode,
            },
        },
    )))
}

fn parse_css_has_data_filter(
    selector: &str,
    mode: CssTextFilterMode,
) -> Result<CssHasDataFilter, String> {
    let mut data_filter_mode = CssDataFilterMode::Contains;
    let (suffix, close_index) = if let Some(suffix) = find_css_not_data_filter_suffix(selector) {
        data_filter_mode = CssDataFilterMode::NotContains;
        (suffix, selector.len() - 2)
    } else if let Some(suffix) = find_css_data_filter_suffix(selector) {
        (suffix, selector.len() - 1)
    } else {
        return Err(format!(
            "CSS :has() compatibility requires :containsData() in `{selector}`"
        ));
    };

    let base = selector[..suffix.open_index].trim();
    let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
    if argument.is_empty() {
        return Err(format!("CSS {} requires text", suffix.name));
    }

    let (selector, direct_child, result_filters, parent_filter) =
        parse_css_has_base_selector(base)?;

    Ok(CssHasDataFilter {
        selector,
        direct_child,
        result_filters,
        parent_filter,
        data_filter: CssDataFilter {
            value: parse_css_text_filter_argument(argument, suffix.name)?,
            mode: data_filter_mode,
        },
        mode,
    })
}

fn parse_css_has_selector_filter(
    selector: &str,
    mode: CssTextFilterMode,
) -> Result<CssHasSelectorFilter, String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err("CSS :has() requires selector".to_string());
    }
    let mut selector = selector;
    let mut nested_filter = None;
    if let Some(suffix) = find_css_not_has_filter_suffix(selector) {
        let close_index = selector.len() - 2;
        let base = selector[..suffix.open_index].trim();
        let inner = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
        if inner.is_empty() {
            return Err(format!("CSS {} requires selector", suffix.name));
        }
        nested_filter = Some(parse_css_nested_has_filter(
            inner,
            CssTextFilterMode::NotContains,
        )?);
        selector = base;
    } else if let Some(suffix) = find_css_has_filter_suffix(selector) {
        let close_index = selector.len() - 1;
        let base = selector[..suffix.open_index].trim();
        let inner = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
        if inner.is_empty() {
            return Err(format!("CSS {} requires selector", suffix.name));
        }
        nested_filter = Some(parse_css_nested_has_filter(
            inner,
            CssTextFilterMode::Contains,
        )?);
        selector = base;
    }
    let (selector, direct_child, result_filters, parent_filter) =
        parse_css_has_base_selector(selector)?;

    Ok(CssHasSelectorFilter {
        selector,
        direct_child,
        result_filters,
        parent_filter,
        nested_filter,
        mode,
    })
}

fn parse_css_nested_has_filter(
    selector: &str,
    mode: CssTextFilterMode,
) -> Result<CssNestedHasFilter, String> {
    let selector = selector.trim();
    if find_css_not_data_filter_suffix(selector).is_some()
        || find_css_data_filter_suffix(selector).is_some()
    {
        return Ok(CssNestedHasFilter::Data(parse_css_has_data_filter(
            selector, mode,
        )?));
    }

    if find_css_not_text_filter_suffix(selector).is_some()
        || find_css_text_filter_suffix(selector).is_some()
    {
        return Ok(CssNestedHasFilter::Text(parse_css_has_text_filter(
            selector, mode,
        )?));
    }

    Ok(CssNestedHasFilter::Selector(Box::new(
        parse_css_has_selector_filter(selector, mode)?,
    )))
}

fn parse_css_has_text_filter(
    selector: &str,
    mode: CssTextFilterMode,
) -> Result<CssHasTextFilter, String> {
    let mut text_filter_mode = CssTextFilterMode::Contains;
    let (suffix, close_index) = if let Some(suffix) = find_css_not_text_filter_suffix(selector) {
        text_filter_mode = CssTextFilterMode::NotContains;
        (suffix, selector.len() - 2)
    } else if let Some(suffix) = find_css_text_filter_suffix(selector) {
        (suffix, selector.len() - 1)
    } else {
        return Err(format!(
            "CSS :has() compatibility requires a supported text filter in `{selector}`"
        ));
    };

    let base = selector[..suffix.open_index].trim();
    let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
    if argument.is_empty() {
        return Err(format!("CSS {} requires text", suffix.name));
    }

    let (selector, direct_child, result_filters, parent_filter) =
        parse_css_has_base_selector(base)?;

    Ok(CssHasTextFilter {
        selector,
        direct_child,
        result_filters,
        parent_filter,
        text_filter: CssTextFilter {
            value: parse_css_text_filter_argument(argument, suffix.name)?,
            scope: suffix.scope,
            matcher: suffix.matcher,
        },
        text_filter_mode,
        mode,
    })
}

fn parse_css_has_base_selector(
    selector: &str,
) -> Result<(String, bool, Vec<CssResultFilter>, Option<CssParentFilter>), String> {
    let selector = selector.trim();
    let direct_child = selector.starts_with('>');
    let selector = if direct_child {
        selector.trim_start_matches('>').trim()
    } else {
        selector
    };
    let (selector, result_filters) = extract_css_result_filters(selector)?;
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
    let selector = if selector.trim().is_empty() {
        "*".to_string()
    } else {
        selector.trim().to_string()
    };

    Ok((selector, direct_child, result_filters, parent_filter))
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

    find_css_has_filter_function(selector)
}

fn find_css_not_has_filter_suffix(selector: &str) -> Option<CssHasFilterSuffix> {
    if !selector.ends_with("))") {
        return None;
    }

    find_css_not_has_filter_function(selector)
}

fn find_css_has_filter_function(selector: &str) -> Option<CssHasFilterSuffix> {
    find_css_function_suffix(selector, ":has(").map(|open_index| CssHasFilterSuffix {
        open_index,
        prefix_len: ":has(".len(),
        name: ":has()",
    })
}

fn find_css_not_has_filter_function(selector: &str) -> Option<CssHasFilterSuffix> {
    find_css_function_suffix(selector, ":not(:has(").map(|open_index| CssHasFilterSuffix {
        open_index,
        prefix_len: ":not(:has(".len(),
        name: ":not(:has())",
    })
}

fn find_first_css_has_filter_for_anchor(
    selector: &str,
) -> Option<(CssHasFilterSuffix, CssTextFilterMode)> {
    let positive = find_first_css_has_filter_function(selector)
        .map(|suffix| (suffix, CssTextFilterMode::Contains));
    let negative = find_first_css_not_has_filter_function(selector)
        .map(|suffix| (suffix, CssTextFilterMode::NotContains));

    match (positive, negative) {
        (Some(positive), Some(negative)) => {
            if positive.0.open_index < negative.0.open_index {
                Some(positive)
            } else {
                Some(negative)
            }
        }
        (Some(positive), None) => Some(positive),
        (None, Some(negative)) => Some(negative),
        (None, None) => None,
    }
}

fn find_first_css_has_filter_function(selector: &str) -> Option<CssHasFilterSuffix> {
    find_first_css_function(selector, ":has(").map(|open_index| CssHasFilterSuffix {
        open_index,
        prefix_len: ":has(".len(),
        name: ":has()",
    })
}

fn find_first_css_not_has_filter_function(selector: &str) -> Option<CssHasFilterSuffix> {
    find_first_css_function(selector, ":not(:has(").map(|open_index| CssHasFilterSuffix {
        open_index,
        prefix_len: ":not(:has(".len(),
        name: ":not(:has())",
    })
}

fn find_css_data_filter_suffix(selector: &str) -> Option<CssDataFilterSuffix> {
    if !selector.ends_with(')') {
        return None;
    }

    find_css_data_filter_function(selector)
}

fn find_css_not_data_filter_suffix(selector: &str) -> Option<CssDataFilterSuffix> {
    if !selector.ends_with("))") {
        return None;
    }

    find_css_not_data_filter_function(selector)
}

fn find_css_data_filter_function(selector: &str) -> Option<CssDataFilterSuffix> {
    find_css_function_suffix(selector, ":containsData(").map(|open_index| CssDataFilterSuffix {
        open_index,
        prefix_len: ":containsData(".len(),
        name: ":containsData()",
    })
}

fn find_css_not_data_filter_function(selector: &str) -> Option<CssDataFilterSuffix> {
    find_css_function_suffix(selector, ":not(:containsData(").map(|open_index| {
        CssDataFilterSuffix {
            open_index,
            prefix_len: ":not(:containsData(".len(),
            name: ":not(:containsData())",
        }
    })
}

fn find_first_css_data_filter_for_anchor(
    selector: &str,
) -> Option<(CssDataFilterSuffix, CssDataFilterMode)> {
    let positive = find_first_css_data_filter_function(selector)
        .map(|suffix| (suffix, CssDataFilterMode::Contains));
    let negative = find_first_css_not_data_filter_function(selector)
        .map(|suffix| (suffix, CssDataFilterMode::NotContains));

    match (positive, negative) {
        (Some(positive), Some(negative)) => {
            if positive.0.open_index < negative.0.open_index {
                Some(positive)
            } else {
                Some(negative)
            }
        }
        (Some(positive), None) => Some(positive),
        (None, Some(negative)) => Some(negative),
        (None, None) => None,
    }
}

fn find_first_css_data_filter_function(selector: &str) -> Option<CssDataFilterSuffix> {
    find_first_css_function(selector, ":containsData(").map(|open_index| CssDataFilterSuffix {
        open_index,
        prefix_len: ":containsData(".len(),
        name: ":containsData()",
    })
}

fn find_first_css_not_data_filter_function(selector: &str) -> Option<CssDataFilterSuffix> {
    find_first_css_function(selector, ":not(:containsData(").map(|open_index| CssDataFilterSuffix {
        open_index,
        prefix_len: ":not(:containsData(".len(),
        name: ":not(:containsData())",
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

fn find_first_css_not_text_filter_function(selector: &str) -> Option<CssTextFilterSuffix> {
    find_first_css_text_filter_function_by(selector, true)
}

fn find_first_css_text_filter_function(selector: &str) -> Option<CssTextFilterSuffix> {
    find_first_css_text_filter_function_by(selector, false)
}

fn find_first_css_text_filter_function_by(
    selector: &str,
    negated: bool,
) -> Option<CssTextFilterSuffix> {
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

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
                if negated {
                    if selector[index..].starts_with(":not(:containsWholeOwnText(") {
                        return Some(CssTextFilterSuffix {
                            open_index: index,
                            prefix_len: ":not(:containsWholeOwnText(".len(),
                            scope: CssTextScope::WholeOwn,
                            matcher: CssTextMatcher::ContainsCaseSensitive,
                            name: ":not(:containsWholeOwnText())",
                        });
                    } else if selector[index..].starts_with(":not(:containsWholeText(") {
                        return Some(CssTextFilterSuffix {
                            open_index: index,
                            prefix_len: ":not(:containsWholeText(".len(),
                            scope: CssTextScope::WholeDescendant,
                            matcher: CssTextMatcher::ContainsCaseSensitive,
                            name: ":not(:containsWholeText())",
                        });
                    } else if selector[index..].starts_with(":not(:containsOwn(") {
                        return Some(CssTextFilterSuffix {
                            open_index: index,
                            prefix_len: ":not(:containsOwn(".len(),
                            scope: CssTextScope::Own,
                            matcher: CssTextMatcher::Contains,
                            name: ":not(:containsOwn())",
                        });
                    } else if selector[index..].starts_with(":not(:contains(") {
                        return Some(CssTextFilterSuffix {
                            open_index: index,
                            prefix_len: ":not(:contains(".len(),
                            scope: CssTextScope::Descendant,
                            matcher: CssTextMatcher::Contains,
                            name: ":not(:contains())",
                        });
                    } else if selector[index..].starts_with(":not(:matchesWholeOwnText(") {
                        return Some(CssTextFilterSuffix {
                            open_index: index,
                            prefix_len: ":not(:matchesWholeOwnText(".len(),
                            scope: CssTextScope::WholeOwn,
                            matcher: CssTextMatcher::Regex,
                            name: ":not(:matchesWholeOwnText())",
                        });
                    } else if selector[index..].starts_with(":not(:matchesWholeText(") {
                        return Some(CssTextFilterSuffix {
                            open_index: index,
                            prefix_len: ":not(:matchesWholeText(".len(),
                            scope: CssTextScope::WholeDescendant,
                            matcher: CssTextMatcher::Regex,
                            name: ":not(:matchesWholeText())",
                        });
                    } else if selector[index..].starts_with(":not(:matchesOwn(") {
                        return Some(CssTextFilterSuffix {
                            open_index: index,
                            prefix_len: ":not(:matchesOwn(".len(),
                            scope: CssTextScope::Own,
                            matcher: CssTextMatcher::Regex,
                            name: ":not(:matchesOwn())",
                        });
                    } else if selector[index..].starts_with(":not(:matches(") {
                        return Some(CssTextFilterSuffix {
                            open_index: index,
                            prefix_len: ":not(:matches(".len(),
                            scope: CssTextScope::Descendant,
                            matcher: CssTextMatcher::Regex,
                            name: ":not(:matches())",
                        });
                    }
                } else if selector[index..].starts_with(":containsWholeOwnText(") {
                    return Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":containsWholeOwnText(".len(),
                        scope: CssTextScope::WholeOwn,
                        matcher: CssTextMatcher::ContainsCaseSensitive,
                        name: ":containsWholeOwnText()",
                    });
                } else if selector[index..].starts_with(":containsWholeText(") {
                    return Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":containsWholeText(".len(),
                        scope: CssTextScope::WholeDescendant,
                        matcher: CssTextMatcher::ContainsCaseSensitive,
                        name: ":containsWholeText()",
                    });
                } else if selector[index..].starts_with(":containsOwn(") {
                    return Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":containsOwn(".len(),
                        scope: CssTextScope::Own,
                        matcher: CssTextMatcher::Contains,
                        name: ":containsOwn()",
                    });
                } else if selector[index..].starts_with(":contains(") {
                    return Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":contains(".len(),
                        scope: CssTextScope::Descendant,
                        matcher: CssTextMatcher::Contains,
                        name: ":contains()",
                    });
                } else if selector[index..].starts_with(":matchesWholeOwnText(") {
                    return Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":matchesWholeOwnText(".len(),
                        scope: CssTextScope::WholeOwn,
                        matcher: CssTextMatcher::Regex,
                        name: ":matchesWholeOwnText()",
                    });
                } else if selector[index..].starts_with(":matchesWholeText(") {
                    return Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":matchesWholeText(".len(),
                        scope: CssTextScope::WholeDescendant,
                        matcher: CssTextMatcher::Regex,
                        name: ":matchesWholeText()",
                    });
                } else if selector[index..].starts_with(":matchesOwn(") {
                    return Some(CssTextFilterSuffix {
                        open_index: index,
                        prefix_len: ":matchesOwn(".len(),
                        scope: CssTextScope::Own,
                        matcher: CssTextMatcher::Regex,
                        name: ":matchesOwn()",
                    });
                } else if selector[index..].starts_with(":matches(") {
                    return Some(CssTextFilterSuffix {
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

fn find_first_css_function(selector: &str, function: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

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
                return Some(index);
            }
            _ => {}
        }
    }

    None
}

fn css_selector_tail_has_top_level_combinator(tail: &str) -> bool {
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;

    for value in tail.chars() {
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
            '>' | '+' | '~' if bracket_depth == 0 && paren_depth == 0 => return true,
            value if value.is_whitespace() && bracket_depth == 0 && paren_depth == 0 => {
                return true;
            }
            _ => {}
        }
    }

    false
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
    evaluate_xpath_expression(input, &rule.expression, &rule.namespaces)
}

fn evaluate_xpath_expression(
    input: &str,
    expression: &str,
    namespaces: &[(String, String)],
) -> RuleResult<Vec<String>> {
    let (expression, replacement) = split_legado_rule_replacement(expression);
    let values = evaluate_xpath_expression_without_replacement(input, expression, namespaces)?;
    Ok(apply_legado_rule_replacement(values, replacement.as_ref()))
}

fn evaluate_xpath_expression_without_replacement(
    input: &str,
    expression: &str,
    namespaces: &[(String, String)],
) -> RuleResult<Vec<String>> {
    if let Some(branches) = split_xpath_top_level_operator(expression, "||")? {
        for branch in branches {
            let results = evaluate_xpath_expression(input, branch, namespaces)?;
            if !results.is_empty() {
                return Ok(results);
            }
        }
        return Ok(Vec::new());
    }

    if let Some(branches) = split_xpath_top_level_operator(expression, "%%")? {
        let mut branch_results = Vec::new();
        for branch in branches {
            let results = evaluate_xpath_expression(input, branch, namespaces)?;
            if !results.is_empty() {
                branch_results.push(results);
            }
        }
        return Ok(zip_json_path_combination_results(branch_results));
    }

    if let Some(branches) = split_xpath_top_level_operator(expression, "&&")? {
        let mut output = Vec::new();
        for branch in branches {
            let results = evaluate_xpath_expression(input, branch, namespaces)?;
            if !results.is_empty() {
                output.extend(results);
            }
        }
        return Ok(output);
    }

    let package = sxd_document::parser::parse(input).map_err(|err| RuleError::XPathInputParse {
        message: err.to_string(),
    })?;
    let document = package.as_document();
    let factory = Factory::new();
    let xpath = factory
        .build(expression)
        .map_err(|err| RuleError::XPathSyntax {
            expression: expression.to_string(),
            message: err.to_string(),
        })?
        .ok_or_else(|| RuleError::XPathSyntax {
            expression: expression.to_string(),
            message: "empty expression".to_string(),
        })?;
    let mut context = Context::new();
    for (prefix, uri) in namespaces {
        context.set_namespace(prefix, uri);
    }

    match xpath
        .evaluate(&context, document.root())
        .map_err(|err| RuleError::XPathEvaluation {
            expression: expression.to_string(),
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

fn split_xpath_top_level_operator<'a>(
    expression: &'a str,
    operator: &str,
) -> RuleResult<Option<Vec<&'a str>>> {
    split_json_path_top_level_operator(expression, operator).map_err(|message| {
        RuleError::XPathSyntax {
            expression: expression.to_string(),
            message,
        }
    })
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

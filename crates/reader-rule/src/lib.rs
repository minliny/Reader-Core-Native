//! Non-JS rule execution primitives for Reader-Core.
//!
//! This crate owns the first native rule semantics before the protocol/runtime
//! layer is ready. The public API is intentionally local to rule execution:
//! callers provide source text and a list of rule steps, and receive a flat list
//! of string results.

use regex::Regex;
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
    /// `[?(@.field == 'value')]` or `[?(@.field >= 1)]` — filter array items by
    /// a scalar field before applying subsequent JSONPath segments.
    Filter(JsonPathFilter),
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
    Compare(JsonPathFilterComparison),
    All(Vec<JsonPathFilter>),
    Any(Vec<JsonPathFilter>),
    Not(Box<JsonPathFilter>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JsonPathFilterComparison {
    left: Vec<String>,
    op: JsonPathFilterOp,
    right: JsonPathFilterOperand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonPathFilterOp {
    Equal,
    NotEqual,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonPathFilterOperand {
    Literal(JsonPathFilterLiteral),
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
    let segments = parse_json_path(&rule.path).map_err(|message| RuleError::JsonPathSyntax {
        path: rule.path.clone(),
        message,
    })?;

    Ok(evaluate_json_path(&value, &segments)
        .into_iter()
        .map(json_value_to_rule_text)
        .collect())
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
                while index < chars.len() && chars[index] != '.' && chars[index] != '[' {
                    index += 1;
                }
                if start == index {
                    return Err("field name expected after `.`".to_string());
                }
                segments.push(JsonPathSegment::Field(chars[start..index].iter().collect()));
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
    let alternatives = split_filter_conditions(expression, "||")?;
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
    let conditions = split_filter_conditions(expression, "&&")?;
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
        let path = parse_filter_path(expression)?;
        return Ok(JsonPathFilter::Exists(path));
    };
    let lhs = expression[..operator_index].trim();
    let rhs = expression[operator_index + operator_len..].trim();

    let left = parse_filter_path(lhs)?;
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

fn split_filter_conditions<'a>(
    expression: &'a str,
    operator: &str,
) -> Result<Vec<&'a str>, String> {
    let operator_marker = operator
        .chars()
        .next()
        .ok_or_else(|| "filter condition operator must not be empty".to_string())?;
    let mut quote = None;
    let mut escaped = false;
    let mut bracket_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut start = 0usize;
    let mut conditions = Vec::new();

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
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            current
                if current == operator_marker
                    && bracket_depth == 0
                    && paren_depth == 0
                    && expression[index..].starts_with(operator) =>
            {
                let condition = expression[start..index].trim();
                if condition.is_empty() {
                    return Err(format!("empty filter condition in `{expression}`"));
                }
                conditions.push(condition);
                start = index + operator.len();
            }
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

        let remaining = &expression[index..];
        if remaining.starts_with("==") {
            return Some((index, 2, JsonPathFilterOp::Equal));
        }
        if remaining.starts_with("!=") {
            return Some((index, 2, JsonPathFilterOp::NotEqual));
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
        return parse_filter_path(rhs).map(JsonPathFilterOperand::Path);
    }

    parse_filter_literal(rhs).map(JsonPathFilterOperand::Literal)
}

fn parse_filter_literal(rhs: &str) -> Result<JsonPathFilterLiteral, String> {
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
    }
}

fn json_path_filter_matches(value: &JsonValue, filter: &JsonPathFilter) -> bool {
    match filter {
        JsonPathFilter::Exists(path) => resolve_filter_path(value, path).is_some(),
        JsonPathFilter::All(filters) => filters
            .iter()
            .all(|filter| json_path_filter_matches(value, filter)),
        JsonPathFilter::Any(filters) => filters
            .iter()
            .any(|filter| json_path_filter_matches(value, filter)),
        JsonPathFilter::Not(filter) => !json_path_filter_matches(value, filter),
        JsonPathFilter::Compare(comparison) => {
            let Some(actual) = resolve_filter_path(value, &comparison.left) else {
                return false;
            };

            match comparison.op {
                JsonPathFilterOp::Equal => filter_values_equal(value, actual, &comparison.right),
                JsonPathFilterOp::NotEqual => {
                    filter_values_comparable(value, actual, &comparison.right)
                        .is_some_and(|equal| !equal)
                }
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

fn filter_values_comparable(
    context: &JsonValue,
    actual: &JsonValue,
    expected: &JsonPathFilterOperand,
) -> Option<bool> {
    match expected {
        JsonPathFilterOperand::Literal(expected) => filter_value_equals_literal(actual, expected),
        JsonPathFilterOperand::Path(path) => {
            let expected = resolve_filter_path(context, path)?;
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
        JsonPathFilterOperand::Path(path) => resolve_filter_path(context, path)?.as_f64(),
        _ => None,
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

    let mut output = Vec::new();
    for element in document.select(&selector) {
        let text = if compat_selector.contains_filter.is_some()
            || matches!(&rule.extraction, CssExtraction::Text)
        {
            Some(normalize_text(
                &element.text().collect::<Vec<_>>().join(" "),
            ))
        } else {
            None
        };
        let own_text = compat_selector
            .contains_filter
            .as_ref()
            .filter(|filter| filter.scope == CssContainsScope::Own)
            .map(|_| normalize_text(&element_own_text(&element)));

        if let Some(filter) = &compat_selector.contains_filter {
            let haystack = match filter.scope {
                CssContainsScope::Descendant => text.as_deref().unwrap_or_default(),
                CssContainsScope::Own => own_text.as_deref().unwrap_or_default(),
            };
            if !css_contains_matches(haystack, &filter.text) {
                continue;
            }
        }

        match &rule.extraction {
            CssExtraction::Text => {
                output.push(text.unwrap_or_default());
            }
            CssExtraction::Attr(attr) => {
                if let Some(value) = element.value().attr(attr) {
                    output.push(value.to_string());
                }
            }
        }
    }

    Ok(output)
}

fn css_contains_matches(text: &str, needle: &str) -> bool {
    text.to_lowercase().contains(&needle.to_lowercase())
}

fn element_own_text(element: &ElementRef<'_>) -> String {
    element
        .children()
        .filter_map(|child| child.value().as_text())
        .map(|text| &**text)
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssCompatSelector {
    selector: String,
    contains_filter: Option<CssContainsFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CssContainsFilter {
    text: String,
    scope: CssContainsScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CssContainsScope {
    Descendant,
    Own,
}

fn parse_css_compat_selector(selector: &str) -> Result<CssCompatSelector, String> {
    let selector = selector.trim();
    let Some(suffix) = find_css_contains_suffix(selector) else {
        return Ok(CssCompatSelector {
            selector: selector.to_string(),
            contains_filter: None,
        });
    };

    let close_index = selector.len() - 1;
    let base = selector[..suffix.open_index].trim();
    let argument = selector[suffix.open_index + suffix.prefix_len..close_index].trim();
    if argument.is_empty() {
        return Err("CSS :contains() requires text".to_string());
    }

    let contains_text = parse_css_contains_argument(argument)?;
    let selector = if base.is_empty() { "*" } else { base };

    Ok(CssCompatSelector {
        selector: selector.to_string(),
        contains_filter: Some(CssContainsFilter {
            text: contains_text,
            scope: suffix.scope,
        }),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CssContainsSuffix {
    open_index: usize,
    prefix_len: usize,
    scope: CssContainsScope,
}

fn find_css_contains_suffix(selector: &str) -> Option<CssContainsSuffix> {
    if !selector.ends_with(')') {
        return None;
    }

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
                if selector[index..].starts_with(":containsOwn(") {
                    candidate = Some(CssContainsSuffix {
                        open_index: index,
                        prefix_len: ":containsOwn(".len(),
                        scope: CssContainsScope::Own,
                    });
                } else if selector[index..].starts_with(":contains(") {
                    candidate = Some(CssContainsSuffix {
                        open_index: index,
                        prefix_len: ":contains(".len(),
                        scope: CssContainsScope::Descendant,
                    });
                }
            }
            _ => {}
        }
    }

    candidate
}

fn parse_css_contains_argument(argument: &str) -> Result<String, String> {
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
                        "unexpected trailing CSS :contains() argument text in `{argument}`"
                    ));
                }
                current => {
                    output.push(current);
                    index += 1;
                }
            }
        }

        return Err(format!(
            "unterminated CSS :contains() argument in `{argument}`"
        ));
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

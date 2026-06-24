//! Non-JS rule execution primitives for Reader-Core.
//!
//! This crate owns the first native rule semantics before the protocol/runtime
//! layer is ready. The public API is intentionally local to rule execution:
//! callers provide source text and a list of rule steps, and receive a flat list
//! of string results.

use regex::Regex;
use scraper::{Html, Selector};
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

            values = next;
            if values.is_empty() {
                break;
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
}

impl RuleStep {
    pub fn regex_capture(pattern: impl Into<String>, group: CaptureGroup) -> Self {
        Self::RegexExtract(RegexExtractRule::all(pattern, group))
    }

    pub fn regex_capture_first(pattern: impl Into<String>, group: CaptureGroup) -> Self {
        Self::RegexExtract(RegexExtractRule::first(pattern, group))
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
}

impl XPathRule {
    pub fn new(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
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
    Wildcard,
}

fn apply_step(input: &str, step: &RuleStep) -> RuleResult<Vec<String>> {
    match step {
        RuleStep::RegexExtract(rule) => apply_regex_extract(input, rule),
        RuleStep::RegexReplace(rule) => apply_regex_replace(input, rule),
        RuleStep::JsonPath(rule) => apply_json_path(input, rule),
        RuleStep::Css(rule) => apply_css(input, rule),
        RuleStep::XPath(rule) => apply_xpath(input, rule),
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
        let value = match &rule.group {
            CaptureGroup::WholeMatch => captures.get(0),
            CaptureGroup::Index(index) => captures.get(*index),
            CaptureGroup::Name(name) => captures.name(name),
        };

        if let Some(value) = value {
            output.push(value.as_str().to_string());
            if !rule.all {
                break;
            }
        }
    }

    Ok(output)
}

fn apply_regex_replace(input: &str, rule: &RegexReplaceRule) -> RuleResult<Vec<String>> {
    let regex = Regex::new(&rule.pattern).map_err(|err| RuleError::RegexSyntax {
        pattern: rule.pattern.clone(),
        message: err.to_string(),
    })?;

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

fn parse_json_path_bracket(
    chars: &[char],
    mut index: usize,
) -> Result<(JsonPathSegment, usize), String> {
    if index >= chars.len() {
        return Err("unterminated `[` segment".to_string());
    }

    if chars[index] == '\'' || chars[index] == '"' {
        let quote = chars[index];
        index += 1;
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
                    if chars.get(index) != Some(&']') {
                        return Err("quoted field segment must close with `]`".to_string());
                    }
                    return Ok((JsonPathSegment::Field(field), index + 1));
                }
                current => {
                    field.push(current);
                    index += 1;
                }
            }
        }

        return Err("unterminated quoted field segment".to_string());
    }

    let start = index;
    while index < chars.len() && chars[index] != ']' {
        index += 1;
    }
    if index >= chars.len() {
        return Err("unterminated `[` segment".to_string());
    }

    let token = chars[start..index]
        .iter()
        .collect::<String>()
        .trim()
        .to_string();
    let segment = if token == "*" {
        JsonPathSegment::Wildcard
    } else if let Ok(array_index) = token.parse::<usize>() {
        JsonPathSegment::Index(array_index)
    } else {
        return Err(format!("unsupported bracket segment `{token}`"));
    };

    Ok((segment, index + 1))
}

fn evaluate_json_path<'a>(root: &'a JsonValue, segments: &[JsonPathSegment]) -> Vec<&'a JsonValue> {
    let mut current = vec![root];

    for segment in segments {
        let mut next = Vec::new();

        for value in current {
            match (segment, value) {
                (JsonPathSegment::Field(field), JsonValue::Object(object)) => {
                    if let Some(value) = object.get(field) {
                        next.push(value);
                    }
                }
                (JsonPathSegment::Index(index), JsonValue::Array(array)) => {
                    if let Some(value) = array.get(*index) {
                        next.push(value);
                    }
                }
                (JsonPathSegment::Wildcard, JsonValue::Array(array)) => {
                    next.extend(array);
                }
                (JsonPathSegment::Wildcard, JsonValue::Object(object)) => {
                    next.extend(object.values());
                }
                _ => {}
            }
        }

        current = next;
        if current.is_empty() {
            break;
        }
    }

    current
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
    let selector = Selector::parse(&rule.selector).map_err(|err| RuleError::CssSelectorSyntax {
        selector: rule.selector.clone(),
        message: format!("{err:?}"),
    })?;

    let mut output = Vec::new();
    for element in document.select(&selector) {
        match &rule.extraction {
            CssExtraction::Text => {
                let text = element
                    .text()
                    .collect::<Vec<_>>()
                    .join("")
                    .trim()
                    .to_string();
                output.push(text);
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

fn apply_xpath(input: &str, rule: &XPathRule) -> RuleResult<Vec<String>> {
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
    let context = Context::new();

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

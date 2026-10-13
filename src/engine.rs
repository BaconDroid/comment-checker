//! Domain-agnostic engine primitives. This module is byte-identical in `comment-checker` and
//! `test-checker` (enforced by `parity/check.py`); the domain layer builds on top of it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer, Serialize};
use tree_sitter::{
    Language, ParseOptions, ParseState, Parser, Query, QueryCursor, StreamingIterator, Tree,
};

/// Per-process parse budget: pathological input is aborted instead of hanging the hook.
pub const PARSE_TIME_BUDGET: Duration = Duration::from_millis(50);

/// BDD keywords allow-listed by both filter catalogs.
pub const BDD_KEYWORDS: &[&str] = &[
    "given",
    "when",
    "then",
    "arrange",
    "act",
    "assert",
    "when & then",
    "when&then",
];

/// Linter directive prefixes allow-listed by both filter catalogs.
pub const DIRECTIVE_PREFIXES: &[&str] = &[
    "type:",
    "noqa",
    "pyright:",
    "ruff:",
    "mypy:",
    "pylint:",
    "flake8:",
    "pyre:",
    "pytype:",
    "eslint-disable",
    "eslint-ignore",
    "prettier-ignore",
    "ts-ignore",
    "ts-expect-error",
    "clippy:",
    "allow",
    "deny",
    "warn",
    "forbid",
];

/// The per-language comment query, shared by both repos' detectors.
pub(crate) fn comment_query(name: &str) -> &'static str {
    match name {
        "rust" | "java" | "elm" | "groovy" => "(line_comment) @comment\n(block_comment) @comment",
        "kotlin" => "(line_comment) @comment\n(multiline_comment) @comment",
        "zig" => "(line_comment) @comment",
        _ => "(comment) @comment",
    }
}

thread_local! {
    static PARSERS: RefCell<HashMap<String, Parser>> = RefCell::new(HashMap::new());
}

/// Parse `source` with a per-thread `Parser` cached by language name.
pub fn parse_cached(name: &str, language: &Language, source: &[u8]) -> Option<Tree> {
    PARSERS.with(|parsers| {
        let mut parsers = parsers.borrow_mut();
        if let Some(parser) = parsers.get_mut(name) {
            parse_with_budget(parser, source)
        } else {
            let mut parser = Parser::new();
            if parser.set_language(language).is_err() {
                return None;
            }
            let tree = parse_with_budget(&mut parser, source);
            parsers.insert(name.to_owned(), parser);
            tree
        }
    })
}

fn parse_with_budget(parser: &mut Parser, source: &[u8]) -> Option<Tree> {
    let start = Instant::now();
    let mut progress = |_state: &ParseState| {
        if start.elapsed() > PARSE_TIME_BUDGET {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = ParseOptions::new().progress_callback(&mut progress);
    let len = source.len();
    let mut read = |offset: usize, _: tree_sitter::Point| {
        if offset < len {
            &source[offset..]
        } else {
            Default::default()
        }
    };
    parser.parse_with_options(&mut read, None, Some(options))
}

/// Null-tolerant `serde` fallback: a missing or `null` field becomes `Default`.
pub fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Parsed hook options shared by both repos.
///
/// `min_confidence` is only meaningful for a repo whose findings carry a confidence (currently
/// test-checker); others ignore it. `strip` is accepted but inert (α): the flag never rewrites the
/// file.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub prompt: String,
    pub min_confidence: Option<String>,
    pub strip: bool,
}

/// Hand-rolled argument parser.
///
/// Recognises `--prompt <text>` / `--prompt=<text>`, `--min-confidence <level>` /
/// `--min-confidence=<level>`, and the inert `--strip`; every other argument is ignored.
#[must_use]
pub fn parse_options(arguments: &[String]) -> Options {
    let mut prompt = String::new();
    let mut min_confidence = None;
    let mut strip = false;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        if let Some(value) = argument.strip_prefix("--prompt=") {
            value.clone_into(&mut prompt);
        } else if argument == "--prompt" {
            if let Some(value) = arguments.get(index + 1) {
                prompt.clone_from(value);
                index += 1;
            }
        } else if let Some(value) = argument.strip_prefix("--min-confidence=") {
            min_confidence = Some(value.to_owned());
        } else if argument == "--min-confidence" {
            if let Some(value) = arguments.get(index + 1) {
                min_confidence = Some(value.clone());
                index += 1;
            }
        } else if argument == "--strip" {
            strip = true;
        }
        index += 1;
    }
    Options {
        prompt,
        min_confidence,
        strip,
    }
}

/// Append `text` with XML text escaping.
pub fn push_escaped_text(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
}

/// Append `text` with XML attribute escaping.
pub fn push_escaped_attribute(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
}

/// One capture: `(row, node kind, text)`.
pub type Capture = (usize, &'static str, String);

/// Collect a query's captures as `(row, node kind, text)`.
pub fn capture_texts(query: &Query, tree: &Tree, source: &[u8]) -> Vec<Capture> {
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(0..source.len());
    let mut captures = cursor.captures(query, tree.root_node(), source);
    let mut out = Vec::new();
    captures.advance();
    while let Some((query_match, capture_index)) = captures.get() {
        if let Some(capture) = query_match.captures.get(*capture_index) {
            if let Ok(text) = capture.node.utf8_text(source) {
                out.push((
                    capture.node.start_position().row,
                    capture.node.kind(),
                    text.to_owned(),
                ));
            }
        }
        captures.advance();
    }
    out
}

/// A resolved injection region: the injected language, its text, and the host row it starts on.
#[derive(Clone, Debug)]
pub struct Injection {
    pub language: String,
    pub content: String,
    pub row: usize,
}

/// Resolve one level of injection regions from a pack injection query.
pub fn injection_regions(query: &Query, tree: &Tree, source: &[u8]) -> Vec<Injection> {
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(0..source.len());
    let mut matches = cursor.matches(query, tree.root_node(), source);
    matches.advance();
    let mut regions = Vec::new();
    while let Some(query_match) = matches.get() {
        let mut content: Option<tree_sitter::Node> = None;
        let mut language: Option<&str> = None;
        for setting in query.property_settings(query_match.pattern_index) {
            if setting.key.as_ref() == "injection.language" {
                language = setting.value.as_deref();
            }
        }
        for capture in query_match.captures {
            match capture_names.get(capture.index as usize).copied() {
                Some("injection.content") => content = Some(capture.node),
                Some("injection.language") => {
                    language = capture.node.utf8_text(source).ok();
                }
                _ => {}
            }
        }
        if let (Some(node), Some(injected)) = (content, language) {
            if let Ok(text) = node.utf8_text(source) {
                regions.push(Injection {
                    language: injected.trim().to_owned(),
                    content: text.to_owned(),
                    row: node.start_position().row,
                });
            }
        }
        matches.advance();
    }
    regions
}

/// New (added) line indices in `new_content`, computed with an LCS over trimmed lines.
pub fn added_lines(old_content: &str, new_content: &str) -> HashSet<usize> {
    let old: Vec<&str> = old_content.lines().map(str::trim).collect();
    let new: Vec<&str> = new_content.lines().map(str::trim).collect();
    if old.len().saturating_mul(new.len()) > 4_000_000 {
        let old_set: HashSet<&str> = old.iter().copied().collect();
        return new
            .iter()
            .enumerate()
            .filter(|(_, line)| !old_set.contains(*line))
            .map(|(index, _)| index)
            .collect();
    }

    let (rows, columns) = (new.len(), old.len());
    let mut lengths = vec![vec![0_u32; columns + 1]; rows + 1];
    for row in (0..rows).rev() {
        for column in (0..columns).rev() {
            lengths[row][column] = if new[row] == old[column] {
                lengths[row + 1][column + 1] + 1
            } else {
                lengths[row + 1][column].max(lengths[row][column + 1])
            };
        }
    }
    let mut matched = vec![false; rows];
    let (mut row, mut column) = (0, 0);
    while row < rows && column < columns {
        if new[row] == old[column] {
            matched[row] = true;
            row += 1;
            column += 1;
        } else if lengths[row + 1][column] >= lengths[row][column + 1] {
            row += 1;
        } else {
            column += 1;
        }
    }
    (0..rows).filter(|&index| !matched[index]).collect()
}

/// Whether a line carries the given suppression marker.
pub fn is_line_suppressed(text: &str, marker: &str) -> bool {
    text.to_lowercase().contains(marker)
}

/// Whether any line in `content` carries the given suppression marker.
pub fn is_file_suppressed(content: &str, marker: &str) -> bool {
    content.lines().any(|line| is_line_suppressed(line, marker))
}

/// Keep the first item for each key.
pub fn dedupe_by<T, K, F>(items: Vec<T>, mut key: F) -> Vec<T>
where
    K: Eq + Hash,
    F: FnMut(&T) -> K,
{
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(key(item)))
        .collect()
}

/// Serialize a hook result, falling back to an error payload on failure.
pub fn result_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| fallback_json(&error.to_string()))
}

/// The shared serialization-failure payload.
pub fn fallback_json(error: &str) -> String {
    serde_json::json!({
        "exitCode": 1,
        "message": format!("serialization failed: {error}"),
    })
    .to_string()
}

/// Strip one leading prefix, if present, then trim.
pub fn strip_one_prefix<'a>(text: &'a str, prefixes: &[&str]) -> &'a str {
    let mut normalized = text.trim();
    for prefix in prefixes {
        if let Some(rest) = normalized.strip_prefix(prefix) {
            normalized = rest.trim();
            break;
        }
    }
    normalized
}

/// First byte of `text`, lowercased (used to bucket the agent-memo catalog).
pub fn first_ascii_lowercase_byte(text: &str) -> Option<u8> {
    text.as_bytes().first().map(u8::to_ascii_lowercase)
}

/// ASCII-case-insensitive `starts_with`.
pub fn starts_with_ignore_ascii_case(text: &str, prefix: &str) -> bool {
    text.as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}

/// Whether `text` starts with `first` followed by any of `seconds` (case-insensitive).
pub fn starts_with_word_pair(text: &str, first: &str, seconds: &[&str]) -> bool {
    let Some(rest) = strip_prefix_ignore_ascii_case(text, first) else {
        return false;
    };
    let rest = rest.trim_start();
    seconds
        .iter()
        .any(|second| starts_with_ignore_ascii_case(rest, second))
}

/// `starts_with_ignore_ascii_case`, returning the remainder.
pub fn strip_prefix_ignore_ascii_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    starts_with_ignore_ascii_case(text, prefix).then(|| &text[prefix.len()..])
}

/// Whether `text` contains an alphabetic `a -> b` arrow.
pub fn has_ascii_arrow(text: &str) -> bool {
    let Some((left, right)) = text.split_once("->") else {
        return false;
    };
    let left = left.trim();
    let right = right.trim();
    !left.is_empty()
        && !right.is_empty()
        && left.bytes().all(|byte| byte.is_ascii_alphabetic())
        && right
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic())
}

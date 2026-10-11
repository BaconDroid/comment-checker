//! Domain-agnostic engine primitives. This module is byte-identical in `comment-checker` and
//! `test-checker` (enforced by `parity/check.py`); the domain layer builds on top of it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer};
use tree_sitter::{
    Language, ParseOptions, ParseState, Parser, Query, QueryCursor, StreamingIterator, Tree,
};

/// Per-process parse budget: pathological input is aborted instead of hanging the hook.
pub const PARSE_TIME_BUDGET: Duration = Duration::from_millis(50);

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

/// Hand-rolled `--prompt <text>` / `--prompt=<text>` parser; every other argument is ignored.
pub fn parse_prompt(arguments: &[String]) -> String {
    let mut prompt = String::new();
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
        }
        index += 1;
    }
    prompt
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

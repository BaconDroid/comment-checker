use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use tree_sitter::{ParseOptions, ParseState, Parser, Query, QueryCursor, StreamingIterator};

use crate::language::{LanguageSpec, docstring_prefix, language_for_path};
use crate::models::{CommentInfo, CommentType};

const PARSE_TIME_BUDGET: Duration = Duration::from_millis(50);

struct Runtime {
    language: tree_sitter::Language,
    comment_query: Query,
    docstring_query: Option<Query>,
}

static RUNTIMES: LazyLock<Mutex<HashMap<&'static str, Arc<Runtime>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

thread_local! {
    static PARSERS: RefCell<HashMap<&'static str, Parser>> = RefCell::new(HashMap::new());
}

#[derive(Debug, Default)]
pub struct CommentDetector;

impl CommentDetector {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn detect(
        &self,
        content: &str,
        file_path: &str,
        include_docstrings: bool,
    ) -> Vec<CommentInfo> {
        let Some(spec) = language_for_path(file_path) else {
            return Vec::new();
        };
        let Some(runtime) = runtime_for(spec) else {
            return Vec::new();
        };
        let source = content.as_bytes();
        let Some(tree) = parse_with_runtime(&runtime, spec.name, source) else {
            return Vec::new();
        };

        let mut comments =
            captures_to_comments(&runtime.comment_query, &tree, source, file_path, false, 0);
        if include_docstrings && !tree.root_node().has_error() {
            if let Some(docstring_query) = &runtime.docstring_query {
                comments.extend(docstrings_from_query(
                    docstring_query,
                    &tree,
                    source,
                    file_path,
                    spec.name,
                    0,
                ));
            }
        }
        collect_injected_comments(
            spec,
            &tree,
            source,
            file_path,
            include_docstrings,
            &mut comments,
        );
        comments
    }
}

fn runtime_for(spec: LanguageSpec) -> Option<Arc<Runtime>> {
    let Ok(mut runtimes) = RUNTIMES.lock() else {
        return None;
    };
    if let Some(runtime) = runtimes.get(spec.name) {
        return Some(Arc::clone(runtime));
    }

    let language = tree_sitter_language_pack::get_language(spec.name).ok()?;
    let comment_query = Query::new(&language, spec.comment_query).ok()?;
    let docstring_query = spec
        .docstring_query
        .and_then(|source| Query::new(&language, source).ok());
    let runtime = Arc::new(Runtime {
        language,
        comment_query,
        docstring_query,
    });
    runtimes.insert(spec.name, Arc::clone(&runtime));
    Some(runtime)
}

fn parse_with_runtime(
    runtime: &Runtime,
    language_name: &'static str,
    source: &[u8],
) -> Option<tree_sitter::Tree> {
    PARSERS.with(|parsers| {
        let mut parsers = parsers.borrow_mut();
        if let Some(parser) = parsers.get_mut(language_name) {
            parse_with_budget(parser, source)
        } else {
            let mut parser = Parser::new();
            if parser.set_language(&runtime.language).is_err() {
                return None;
            }
            let tree = parse_with_budget(&mut parser, source);
            parsers.insert(language_name, parser);
            tree
        }
    })
}

fn parse_with_budget(parser: &mut Parser, source: &[u8]) -> Option<tree_sitter::Tree> {
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

fn captures_to_comments(
    query: &Query,
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    force_docstring: bool,
    row_offset: usize,
) -> Vec<CommentInfo> {
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(0..source.len());
    let mut captures = cursor.captures(query, tree.root_node(), source);
    let mut comments = Vec::new();
    captures.advance();
    while let Some((query_match, capture_index)) = captures.get() {
        let Some(capture) = query_match.captures.get(*capture_index) else {
            captures.advance();
            continue;
        };
        let node = capture.node;
        let Ok(text) = node.utf8_text(source) else {
            captures.advance();
            continue;
        };
        let comment_type = if force_docstring {
            CommentType::Docstring
        } else {
            determine_comment_type(text, node.kind())
        };
        comments.push(CommentInfo::new(
            text,
            node.start_position().row + 1 + row_offset,
            file_path,
            comment_type,
            force_docstring,
        ));
        captures.advance();
    }
    comments
}

fn docstrings_from_query(
    query: &Query,
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language_name: &str,
    row_offset: usize,
) -> Vec<CommentInfo> {
    let mut docstrings = captures_to_comments(query, tree, source, file_path, true, row_offset);
    if let Some(prefix) = docstring_prefix(language_name) {
        docstrings.retain(|comment| comment.text.trim_start().starts_with(prefix));
    }
    docstrings
}

fn collect_injected_comments(
    spec: LanguageSpec,
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    include_docstrings: bool,
    comments: &mut Vec<CommentInfo>,
) {
    let Some(injection_source) = tree_sitter_language_pack::get_injections_query(spec.name) else {
        return;
    };
    let Ok(language) = tree_sitter_language_pack::get_language(spec.name) else {
        return;
    };
    let Ok(query) = Query::new(&language, injection_source) else {
        return;
    };
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(0..source.len());
    let mut matches = cursor.matches(&query, tree.root_node(), source);
    matches.advance();
    while let Some(query_match) = matches.get() {
        let mut content: Option<tree_sitter::Node> = None;
        let mut injected_language: Option<&str> = None;
        for setting in query.property_settings(query_match.pattern_index) {
            if setting.key.as_ref() == "injection.language" {
                injected_language = setting.value.as_deref();
            }
        }
        for capture in query_match.captures {
            match capture_names.get(capture.index as usize).copied() {
                Some("injection.content") => content = Some(capture.node),
                Some("injection.language") => {
                    injected_language = capture.node.utf8_text(source).ok();
                }
                _ => {}
            }
        }
        if let (Some(content), Some(name)) = (content, injected_language) {
            if let Ok(text) = content.utf8_text(source) {
                comments.extend(detect_injected(
                    name.trim(),
                    text,
                    file_path,
                    content.start_position().row,
                    include_docstrings,
                ));
            }
        }
        matches.advance();
    }
}

fn detect_injected(
    name: &str,
    content: &str,
    file_path: &str,
    row_offset: usize,
    include_docstrings: bool,
) -> Vec<CommentInfo> {
    let Ok(language) = tree_sitter_language_pack::get_language(name) else {
        return Vec::new();
    };
    let Ok(comment_query) = Query::new(&language, crate::language::comment_query(name)) else {
        return Vec::new();
    };
    let docstring_query = crate::language::docstring_query(name)
        .and_then(|source| Query::new(&language, source).ok());
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let source = content.as_bytes();
    let Some(tree) = parse_with_budget(&mut parser, source) else {
        return Vec::new();
    };
    let mut comments =
        captures_to_comments(&comment_query, &tree, source, file_path, false, row_offset);
    if include_docstrings && !tree.root_node().has_error() {
        if let Some(docstring_query) = &docstring_query {
            comments.extend(docstrings_from_query(
                docstring_query,
                &tree,
                source,
                file_path,
                name,
                row_offset,
            ));
        }
    }
    comments
}

fn determine_comment_type(text: &str, node_kind: &str) -> CommentType {
    let stripped = text.trim();
    if node_kind == "line_comment" {
        return CommentType::Line;
    }
    if node_kind == "block_comment" || node_kind == "multiline_comment" {
        return CommentType::Block;
    }
    if stripped.starts_with("//") || stripped.starts_with('#') {
        return CommentType::Line;
    }
    if stripped.starts_with("/*") || stripped.starts_with("<!--") || stripped.starts_with("--") {
        return CommentType::Block;
    }
    CommentType::Line
}

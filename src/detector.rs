use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::language::{LanguageSpec, language_for_path};
use crate::models::{CommentInfo, CommentType};

const COMMENT_KINDS: &[&str] = &[
    "comment",
    "line_comment",
    "block_comment",
    "multiline_comment",
];

struct Runtime {
    language: tree_sitter::Language,
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
        let Some(tree) = parse_with_runtime(&runtime, spec.name, content.as_bytes()) else {
            return Vec::new();
        };

        let mut comments = collect_comments(&tree, content.as_bytes(), file_path);
        if include_docstrings {
            if let Some(docstring_query) = &runtime.docstring_query {
                comments.extend(captures_to_docstrings(
                    docstring_query,
                    &tree,
                    content.as_bytes(),
                    file_path,
                ));
            }
        }
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
    let docstring_query = spec
        .docstring_query
        .and_then(|source| Query::new(&language, source).ok());
    let runtime = Arc::new(Runtime {
        language,
        docstring_query,
    });
    runtimes.insert(spec.name, Arc::clone(&runtime));
    Some(runtime)
}

fn parse_with_runtime(
    runtime: &Runtime,
    language_name: &'static str,
    source: &[u8],
) -> Option<Tree> {
    PARSERS.with(|parsers| {
        let mut parsers = parsers.borrow_mut();
        if let Some(parser) = parsers.get_mut(language_name) {
            parser.parse(source, None)
        } else {
            let mut parser = Parser::new();
            parser.set_language(&runtime.language).ok()?;
            let tree = parser.parse(source, None);
            parsers.insert(language_name, parser);
            tree
        }
    })
}

fn collect_comments(tree: &Tree, source: &[u8], file_path: &str) -> Vec<CommentInfo> {
    let mut comments = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if COMMENT_KINDS.contains(&node.kind())
            && let Ok(text) = node.utf8_text(source)
        {
            let comment_type = determine_comment_type(text, node.kind());
            comments.push(CommentInfo::new(
                text,
                node.start_position().row + 1,
                file_path,
                comment_type,
                false,
            ));
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return comments;
            }
        }
    }
}

fn captures_to_docstrings(
    query: &Query,
    tree: &Tree,
    source: &[u8],
    file_path: &str,
) -> Vec<CommentInfo> {
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(query, tree.root_node(), source);
    let mut docstrings = Vec::new();
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
        docstrings.push(CommentInfo::new(
            text,
            node.start_position().row + 1,
            file_path,
            CommentType::Docstring,
            true,
        ));
        captures.advance();
    }
    docstrings
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

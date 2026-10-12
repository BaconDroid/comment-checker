use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use tree_sitter::Query;

use crate::engine;
use crate::language::{docstring_prefix, language_for_path};
use crate::models::{CommentInfo, CommentType};

struct Runtime {
    name: String,
    language: tree_sitter::Language,
    comment_query: Query,
    docstring_query: Option<Query>,
    injection_query: Option<Query>,
}

static RUNTIMES: LazyLock<Mutex<HashMap<String, Arc<Runtime>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
        let Some(name) = language_for_path(file_path) else {
            return Vec::new();
        };
        let Some(runtime) = runtime_by_name(name) else {
            return Vec::new();
        };
        let source = content.as_bytes();
        let Some(tree) = parse_with_runtime(&runtime, source) else {
            return Vec::new();
        };

        let mut comments = extract(&runtime, &tree, content, file_path, 0, include_docstrings);
        collect_injected_comments(
            &runtime,
            &tree,
            source,
            file_path,
            include_docstrings,
            &mut comments,
        );
        // An injected region can overlap what the host comment query already matched.
        engine::dedupe_by(comments, |comment| {
            (comment.line_number, comment.normalized_text())
        })
    }

    /// Analyze an edit: report only comments newly introduced by `new_string`.
    #[must_use]
    pub fn detect_edit(
        &self,
        old_string: &str,
        new_string: &str,
        file_path: &str,
        include_docstrings: bool,
    ) -> Vec<CommentInfo> {
        let comments = self.detect(new_string, file_path, include_docstrings);
        if old_string.is_empty() {
            return comments;
        }
        // Keep only comments that sit on a line the edit actually added, using an LCS over the old
        // and new lines. This is accurate for repeated/moved text, where a plain set-membership
        // check would drop a genuinely new comment whose text already appeared elsewhere.
        let added = engine::added_lines(old_string, new_string);
        comments
            .into_iter()
            .filter(|comment| added.contains(&comment.line_number.saturating_sub(1)))
            .collect()
    }
}

fn runtime_by_name(name: &str) -> Option<Arc<Runtime>> {
    let Ok(mut runtimes) = RUNTIMES.lock() else {
        return None;
    };
    if let Some(runtime) = runtimes.get(name) {
        return Some(Arc::clone(runtime));
    }

    let language = tree_sitter_language_pack::get_language(name).ok()?;
    let comment_query = Query::new(&language, engine::comment_query(name)).ok()?;
    let docstring_query = crate::language::docstring_query(name)
        .and_then(|source| Query::new(&language, source).ok());
    let injection_query = tree_sitter_language_pack::get_injections_query(name)
        .and_then(|source| Query::new(&language, source).ok());
    let runtime = Arc::new(Runtime {
        name: name.to_owned(),
        language,
        comment_query,
        docstring_query,
        injection_query,
    });
    runtimes.insert(name.to_owned(), Arc::clone(&runtime));
    Some(runtime)
}

fn parse_with_runtime(runtime: &Runtime, source: &[u8]) -> Option<tree_sitter::Tree> {
    engine::parse_cached(&runtime.name, &runtime.language, source)
}

fn extract(
    runtime: &Runtime,
    tree: &tree_sitter::Tree,
    content: &str,
    file_path: &str,
    row_offset: usize,
    include_docstrings: bool,
) -> Vec<CommentInfo> {
    let source = content.as_bytes();
    let mut comments = captures_to_comments(
        &runtime.comment_query,
        tree,
        source,
        file_path,
        false,
        row_offset,
    );
    if include_docstrings {
        if let Some(docstring_query) = &runtime.docstring_query {
            if !tree.root_node().has_error() {
                comments.extend(docstrings_from_query(
                    docstring_query,
                    tree,
                    source,
                    file_path,
                    &runtime.name,
                    row_offset,
                ));
            }
        } else if runtime.name == "python" {
            comments.extend(python_docstrings(content, file_path, row_offset));
        }
    }
    comments
}

fn detects_injected(
    runtime: &Runtime,
    content: &str,
    file_path: &str,
    row_offset: usize,
    include_docstrings: bool,
) -> Vec<CommentInfo> {
    let source = content.as_bytes();
    let Some(tree) = parse_with_runtime(runtime, source) else {
        return Vec::new();
    };
    extract(
        runtime,
        &tree,
        content,
        file_path,
        row_offset,
        include_docstrings,
    )
}

fn captures_to_comments(
    query: &Query,
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    force_docstring: bool,
    row_offset: usize,
) -> Vec<CommentInfo> {
    engine::capture_texts(query, tree, source)
        .into_iter()
        .map(|(row, node_kind, text)| {
            let comment_type = if force_docstring {
                CommentType::Docstring
            } else {
                determine_comment_type(&text, node_kind)
            };
            CommentInfo::new(
                text,
                row + 1 + row_offset,
                file_path,
                comment_type,
                force_docstring,
            )
        })
        .collect()
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

fn python_docstrings(content: &str, file_path: &str, row_offset: usize) -> Vec<CommentInfo> {
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();
            (trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")).then(|| {
                CommentInfo::new(
                    trimmed,
                    index + 1 + row_offset,
                    file_path,
                    CommentType::Docstring,
                    true,
                )
            })
        })
        .collect()
}

fn collect_injected_comments(
    runtime: &Runtime,
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    include_docstrings: bool,
    comments: &mut Vec<CommentInfo>,
) {
    let Some(query) = &runtime.injection_query else {
        return;
    };
    for injection in engine::injection_regions(query, tree, source) {
        let Some(inner) = runtime_by_name(&injection.language) else {
            continue;
        };
        comments.extend(detects_injected(
            &inner,
            &injection.content,
            file_path,
            injection.row,
            include_docstrings,
        ));
    }
}

fn determine_comment_type(text: &str, node_kind: &str) -> CommentType {
    let stripped = text.trim();
    if node_kind == "line_comment" {
        return CommentType::Line;
    }
    if node_kind == "block_comment" || node_kind == "multiline_comment" {
        return CommentType::Block;
    }
    if stripped.starts_with("//") || stripped.starts_with('#') || stripped.starts_with("--") {
        return CommentType::Line;
    }
    if stripped.starts_with("/*") || stripped.starts_with("(*") || stripped.starts_with("<!--") {
        return CommentType::Block;
    }
    CommentType::Line
}

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use tree_sitter::Query;

use crate::engine;
use crate::language::{docstring_prefix, language_for_path};
use crate::models::{CommentInfo, CommentType};

/// Why detection produced no comments. The hook maps these to visible outcomes instead of
/// reporting a pass. Crate-private: the public API keeps its historical `Vec` shape, and the
/// fail-open path is [`CommentDetector::detect_checked`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DetectError {
    /// `engine::parse_cached` stopped at `engine::PARSE_TIME_BUDGET` without a tree.
    BudgetExceeded,
    /// A runtime, query, or parser failed in a way the hook cannot recover from.
    Internal(String),
}

/// The detector's findings, or the reason it could not produce them.
pub(crate) type DetectResult = Result<Vec<CommentInfo>, DetectError>;

impl fmt::Display for DetectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BudgetExceeded => formatter.write_str("parse budget exceeded"),
            Self::Internal(reason) => formatter.write_str(reason),
        }
    }
}

/// Test-only switch proving the internal-failure path end to end. Production builds have no way to
/// set it: there is no environment variable or flag.
#[cfg(test)]
static FORCE_INTERNAL_FAILURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The reason reported while [`FORCE_INTERNAL_FAILURE`] is on.
#[cfg(test)]
pub(crate) const FORCED_INTERNAL_FAILURE_REASON: &str = "forced internal failure";

/// Turn the test-only internal-failure switch on or off.
#[cfg(test)]
pub(crate) fn set_forced_internal_failure(enabled: bool) {
    FORCE_INTERNAL_FAILURE.store(enabled, std::sync::atomic::Ordering::SeqCst);
}

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

    /// Detect comments, keeping the historical `Vec` return for test and bench callers.
    ///
    /// An internal failure yields an empty list here, so this is **not** safe for the hook: the
    /// hook path uses [`Self::detect_checked`] to turn a failure into a visible `did not run`.
    #[must_use]
    pub fn detect(
        &self,
        content: &str,
        file_path: &str,
        include_docstrings: bool,
    ) -> Vec<CommentInfo> {
        Self::detect_checked(content, file_path, include_docstrings).unwrap_or_default()
    }

    /// Detect comments, reporting why detection could not run.
    pub(crate) fn detect_checked(
        content: &str,
        file_path: &str,
        include_docstrings: bool,
    ) -> DetectResult {
        let Some(name) = language_for_path(file_path) else {
            return Ok(Vec::new());
        };
        let runtime = runtime_by_name(name)?;
        let source = content.as_bytes();
        let tree = parse_with_runtime(&runtime, source, file_path)?;

        let mut comments = extract(&runtime, &tree, content, file_path, 0, include_docstrings);
        collect_injected_comments(
            &runtime,
            &tree,
            source,
            file_path,
            include_docstrings,
            &mut comments,
        )?;
        // An injected region can overlap what the host comment query already matched.
        Ok(engine::dedupe_by(comments, |comment| {
            (comment.line_number, comment.normalized_text())
        }))
    }

    /// Analyze an edit, keeping the historical `Vec` return for test and bench callers.
    ///
    /// As with [`Self::detect`], an internal failure yields an empty list; the hook calls
    /// [`Self::detect_edit_checked`].
    #[must_use]
    pub fn detect_edit(
        &self,
        old_string: &str,
        new_string: &str,
        file_path: &str,
        include_docstrings: bool,
    ) -> Vec<CommentInfo> {
        Self::detect_edit_checked(old_string, new_string, file_path, include_docstrings)
            .unwrap_or_default()
    }

    /// Analyze an edit: report only comments newly introduced by `new_string`, or why detection
    /// could not run.
    pub(crate) fn detect_edit_checked(
        old_string: &str,
        new_string: &str,
        file_path: &str,
        include_docstrings: bool,
    ) -> DetectResult {
        let comments = Self::detect_checked(new_string, file_path, include_docstrings)?;
        if old_string.is_empty() {
            return Ok(comments);
        }
        // Keep only comments that sit on a line the edit actually added, using an LCS over the old
        // and new lines. This is accurate for repeated/moved text, where a plain set-membership
        // check would drop a genuinely new comment whose text already appeared elsewhere.
        let added = engine::added_lines(old_string, new_string);
        Ok(comments
            .into_iter()
            .filter(|comment| added.contains(&comment.line_number.saturating_sub(1)))
            .collect())
    }
}

fn runtime_by_name(name: &str) -> Result<Arc<Runtime>, DetectError> {
    #[cfg(test)]
    if FORCE_INTERNAL_FAILURE.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(DetectError::Internal(
            FORCED_INTERNAL_FAILURE_REASON.to_owned(),
        ));
    }

    let Ok(mut runtimes) = RUNTIMES.lock() else {
        return Err(DetectError::Internal(
            "language runtime unavailable: runtime cache poisoned".to_owned(),
        ));
    };
    if let Some(runtime) = runtimes.get(name) {
        return Ok(Arc::clone(runtime));
    }

    let language = tree_sitter_language_pack::get_language(name)
        .map_err(|_| DetectError::Internal(format!("language runtime unavailable: {name}")))?;
    let comment_query = Query::new(&language, engine::comment_query(name))
        .map_err(|_| DetectError::Internal(format!("query compilation failed: {name}")))?;
    // Optional queries stay best-effort: a language without docstrings or injections parses fine
    // without them.
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
    Ok(runtime)
}

/// Parse `source`, telling a budget stop apart from a hard parser failure.
///
/// `engine::parse_cached` returns `None` for both, and a precise budget flag would change the
/// shared `engine.rs`; instead the elapsed time is the discriminator. A parse that fails at or
/// after the budget is treated as a budget stop (matching engine's own `>` comparison), and one
/// that fails sooner is an internal parse failure. A hard failure that takes longer than the
/// budget is therefore misreported as a skip: an approximation, not a guarantee.
fn parse_with_runtime(
    runtime: &Runtime,
    source: &[u8],
    file_path: &str,
) -> Result<tree_sitter::Tree, DetectError> {
    let started = Instant::now();
    match engine::parse_cached(&runtime.name, &runtime.language, source) {
        Some(tree) => Ok(tree),
        None if started.elapsed() >= engine::PARSE_TIME_BUDGET => Err(DetectError::BudgetExceeded),
        None => Err(DetectError::Internal(format!("parse failed: {file_path}"))),
    }
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
) -> DetectResult {
    let source = content.as_bytes();
    let tree = parse_with_runtime(runtime, source, file_path)?;
    Ok(extract(
        runtime,
        &tree,
        content,
        file_path,
        row_offset,
        include_docstrings,
    ))
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
) -> Result<(), DetectError> {
    let Some(query) = &runtime.injection_query else {
        return Ok(());
    };
    for injection in engine::injection_regions(query, tree, source) {
        let Some(inner) = injected_runtime(&injection.language)? else {
            continue;
        };
        comments.extend(detects_injected(
            &inner,
            &injection.content,
            file_path,
            injection.row,
            include_docstrings,
        )?);
    }
    Ok(())
}

/// The runtime for a language an injection query names, or `None` when the pack has no grammar for
/// it. Host grammars inject comment bodies as `comment`/`jsdoc`, which the path resolver never
/// selects; skipping those regions is not a failure, while a supported grammar's runtime or parse
/// failure is.
fn injected_runtime(name: &str) -> Result<Option<Arc<Runtime>>, DetectError> {
    if tree_sitter_language_pack::get_language(name).is_err() {
        return Ok(None);
    }
    runtime_by_name(name).map(Some)
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

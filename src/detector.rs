use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::ops::ControlFlow;
use std::sync::{Arc, LazyLock, Mutex};

use tree_sitter::{ParseOptions, ParseState, Parser, Query, QueryCursor, StreamingIterator};

use crate::engine;
use crate::language::{docstring_prefix, language_for_path};
use crate::models::{CommentInfo, CommentType};

/// Why detection produced no comments. The hook maps these to visible outcomes instead of
/// reporting a pass. Crate-private: the public API keeps its historical `Vec` shape, and the
/// fail-open path is [`CommentDetector::detect_checked`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DetectError {
    /// The parse reached [`PARSE_OPERATION_BUDGET`] without producing a tree.
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

/// Test-only switch proving the budget path end to end. Production builds have no way to set it:
/// there is no environment variable or flag.
#[cfg(test)]
static FORCE_BUDGET_EXCEEDED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Turn the test-only budget switch on or off.
#[cfg(test)]
pub(crate) fn set_forced_budget_exceeded(enabled: bool) {
    FORCE_BUDGET_EXCEEDED.store(enabled, std::sync::atomic::Ordering::SeqCst);
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
        reset_parse_budget();
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

/// Progress callbacks (each covering a fixed 100 parser operations) allowed per top-level
/// detection call.
///
/// The shared engine aborts on a 50 ms wall clock, so identical bytes near that boundary are
/// sometimes analyzed and sometimes skipped depending on machine load. A budget of parser
/// operations is a deterministic function of the input: the same bytes always consume the same
/// number of operations and therefore always reach the same decision. Measured on this binary,
/// 5,000 callbacks completes the largest real source payload in the corpus (a 484 KB Python file at
/// 3,909) while staying below a 407 KB minified JavaScript bundle (6,500) and a 1,067 KB one
/// (18,577), which are stable skips.
const PARSE_OPERATION_BUDGET: u64 = 5_000;

thread_local! {
    static PARSE_OPERATIONS_USED: Cell<u64> = const { Cell::new(0) };
}

/// Start a new budget window. Every detection call begins one, so the parses of one call share the
/// budget while a later call starts fresh.
fn reset_parse_budget() {
    PARSE_OPERATIONS_USED.with(|used| used.set(0));
}

/// Parse `source` against a deterministic operation budget.
///
/// The engine's wall-clock abort is deliberately not used here: its outcome depends on host load,
/// so the same bytes could analyze on one run and skip on the next. `engine.rs` is byte-identical
/// with the sibling hook and cannot grow the abort flag this needs, so the parse is performed here
/// with the same `tree_sitter` API and a fresh parser. A fresh parser also avoids the engine cache's
/// resume-after-abort state, where a cancelled parse leaves the cached parser mid-document.
///
/// `None` with the abort flag set is a deterministic [`DetectError::BudgetExceeded`]; any other
/// `None` is a genuine parse failure.
fn parse_with_runtime(
    runtime: &Runtime,
    source: &[u8],
    file_path: &str,
) -> Result<tree_sitter::Tree, DetectError> {
    #[cfg(test)]
    if FORCE_BUDGET_EXCEEDED.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(DetectError::BudgetExceeded);
    }

    let used = PARSE_OPERATIONS_USED.with(Cell::get);
    if used >= PARSE_OPERATION_BUDGET {
        return Err(DetectError::BudgetExceeded);
    }
    let mut parser = Parser::new();
    parser
        .set_language(&runtime.language)
        .map_err(|_| DetectError::Internal(format!("parse failed: {file_path}")))?;
    let ticks = Cell::new(0_u64);
    let aborted = Cell::new(false);
    let len = source.len();
    let tree = {
        let mut progress = |_state: &ParseState| {
            ticks.set(ticks.get() + 1);
            if used + ticks.get() >= PARSE_OPERATION_BUDGET {
                aborted.set(true);
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        let options = ParseOptions::new().progress_callback(&mut progress);
        let mut read = |offset: usize, _: tree_sitter::Point| {
            if offset < len {
                &source[offset..]
            } else {
                Default::default()
            }
        };
        parser.parse_with_options(&mut read, None, Some(options))
    };
    PARSE_OPERATIONS_USED.with(|used_cell| used_cell.set(used + ticks.get()));
    match tree {
        Some(tree) => Ok(tree),
        None if aborted.get() => Err(DetectError::BudgetExceeded),
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

/// Query captures as `(row, node kind, text, start byte, end byte)`.
///
/// The range is kept here because the removal pass edits the exact node while `engine::capture_texts`
/// only feeds reporting.
fn capture_ranges(
    query: &Query,
    tree: &tree_sitter::Tree,
    source: &[u8],
) -> Vec<(usize, &'static str, String, usize, usize)> {
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
                    capture.node.start_byte(),
                    capture.node.end_byte(),
                ));
            }
        }
        captures.advance();
    }
    out
}

fn captures_to_comments(
    query: &Query,
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    force_docstring: bool,
    row_offset: usize,
) -> Vec<CommentInfo> {
    capture_ranges(query, tree, source)
        .into_iter()
        .map(|(row, node_kind, text, start, end)| {
            let comment_type = if force_docstring {
                CommentType::Docstring
            } else {
                determine_comment_type(&text, node_kind)
            };
            let mut comment = CommentInfo::new(
                text,
                row + 1 + row_offset,
                file_path,
                comment_type,
                force_docstring,
            );
            // An injected range indexes the injected text, not the host file the removal pass
            // edits, so only a host capture carries a usable span.
            comment.byte_span = (row_offset == 0).then_some((start, end));
            comment
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
    let mut offset = 0;
    content
        .split_inclusive('\n')
        .enumerate()
        .filter_map(|(index, line)| {
            let body = line.strip_suffix('\n').unwrap_or(line);
            let body = body.strip_suffix('\r').unwrap_or(body);
            let trimmed = body.trim_start();
            let comment =
                (trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''")).then(|| {
                    let mut comment = CommentInfo::new(
                        trimmed,
                        index + 1 + row_offset,
                        file_path,
                        CommentType::Docstring,
                        true,
                    );
                    if row_offset == 0 {
                        let start = offset + (body.len() - trimmed.len());
                        comment.byte_span = Some((start, start + trimmed.len()));
                    }
                    comment
                });
            offset += line.len();
            comment
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

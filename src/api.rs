use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::detector::{CommentDetector, DetectError};
use crate::engine::{self, null_default};
use crate::filters::{SuppressionFilter, apply_filters};
use crate::language::language_for_path;
use crate::models::{CommentInfo, Edit};
use crate::output::format_hook_message;

pub const EXIT_PASS: i32 = 0;
pub(crate) const EXIT_INTERNAL_FAILURE: i32 = 1;
pub const EXIT_BLOCK: i32 = 2;

/// Largest content payload the hook will inspect. A bigger write, edit, or multipart edit is
/// skipped visibly instead of spending the parse budget on a file the hook cannot finish.
pub(crate) const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;

/// What the hook actually did. `Skipped` and `Failed` are distinct: a skip means nothing needed
/// checking, while a failure means nothing was checked at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HookOutcome {
    /// Nothing to report.
    Clean,
    /// Problematic comments were found; the message is the existing block payload.
    Findings(String),
    /// The input never reached the detector, or the parser hit its budget.
    Skipped(String),
    /// An internal failure stopped the check from running.
    Failed(String),
}

impl HookOutcome {
    #[must_use]
    pub(crate) const fn exit_code(&self) -> i32 {
        match self {
            Self::Clean | Self::Skipped(_) => EXIT_PASS,
            Self::Findings(_) => EXIT_BLOCK,
            Self::Failed(_) => EXIT_INTERNAL_FAILURE,
        }
    }

    #[must_use]
    pub(crate) fn message(&self) -> String {
        match self {
            Self::Clean => {
                "[check-comments] Success: No problematic comments/docstrings found\n".to_owned()
            }
            Self::Findings(message) => message.clone(),
            Self::Skipped(reason) => format!("[check-comments] Skipping: {reason}\n"),
            Self::Failed(reason) => {
                format!("[check-comments] did not run — nothing checked this write.\n{reason}\n")
            }
        }
    }

    /// The wire result `src/cli.rs` reads. The shared CLI depends on `exit_code`/`message` only.
    #[must_use]
    pub(crate) fn into_result(self) -> HookCheckResult {
        HookCheckResult {
            exit_code: self.exit_code(),
            message: self.message(),
        }
    }
}

/// The hook tool that produced a change. Claude Code sends `Write`/`Edit`/`MultiEdit`; the
/// `OpenCode` plugin normalizes to the same names but also forwards `apply_patch`, so matching is
/// case-insensitive and `apply_patch` behaves like an edit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    Write,
    Edit,
    MultiEdit,
}

fn classify_tool(name: &str) -> Option<ToolKind> {
    if name.eq_ignore_ascii_case("write") {
        Some(ToolKind::Write)
    } else if name.eq_ignore_ascii_case("edit") || name.eq_ignore_ascii_case("apply_patch") {
        Some(ToolKind::Edit)
    } else if name.eq_ignore_ascii_case("multiedit") {
        Some(ToolKind::MultiEdit)
    } else {
        None
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookCheckResult {
    pub exit_code: i32,
    pub message: String,
}

impl HookCheckResult {
    #[must_use]
    pub fn pass(message: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_PASS,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn block(message: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_BLOCK,
            message: message.into(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ToolInput {
    #[serde(default, alias = "filePath", deserialize_with = "null_default")]
    file_path: String,
    #[serde(default, deserialize_with = "null_default")]
    content: String,
    #[serde(default, alias = "newString", deserialize_with = "null_default")]
    new_string: String,
    #[serde(default, alias = "oldString", deserialize_with = "null_default")]
    old_string: String,
    #[serde(default, deserialize_with = "null_default")]
    edits: Vec<Edit>,
}

#[derive(Debug, Deserialize)]
struct HookInput {
    #[serde(default, alias = "toolName", deserialize_with = "null_default")]
    tool_name: String,
    #[serde(default, alias = "toolInput", deserialize_with = "null_default")]
    tool_input: ToolInput,
    #[serde(default, alias = "cwd", deserialize_with = "null_default")]
    cwd: String,
}

#[must_use]
pub fn check_hook_input(input: &str, custom_prompt: &str) -> HookCheckResult {
    check_hook_input_with_options(input, custom_prompt, true)
}

/// Same as [`check_hook_input`], with the `include_docstrings` toggle threaded through the hook
/// boundary so both states are testable.
#[must_use]
pub fn check_hook_input_with_options(
    input: &str,
    custom_prompt: &str,
    include_docstrings: bool,
) -> HookCheckResult {
    if input.is_empty() {
        return skip("No input provided");
    }

    serde_json::from_str::<HookInput>(input).map_or_else(
        |_| skip("Invalid input format"),
        |hook_input| evaluate(&hook_input, custom_prompt, include_docstrings).into_result(),
    )
}

#[must_use]
pub fn check_hook_input_from_reader<R: std::io::Read>(
    reader: R,
    options: &crate::engine::Options,
) -> HookCheckResult {
    serde_json::from_reader::<R, HookInput>(reader).map_or_else(
        |_| skip("Invalid input format"),
        |hook_input| {
            if options.strip {
                evaluate_strip(&hook_input, &options.prompt, true)
            } else {
                evaluate(&hook_input, &options.prompt, true).into_result()
            }
        },
    )
}

fn evaluate(hook_input: &HookInput, custom_prompt: &str, include_docstrings: bool) -> HookOutcome {
    let Some(tool) = classify_tool(&hook_input.tool_name) else {
        return HookOutcome::Skipped("Unsupported tool".to_owned());
    };
    let file_path = hook_input.tool_input.file_path.as_str();
    if file_path.is_empty() {
        return HookOutcome::Skipped("No file path provided".to_owned());
    }
    if language_for_path(file_path).is_none() {
        return HookOutcome::Skipped("Non-code file".to_owned());
    }
    if exceeds_content_limit(tool, hook_input) {
        return HookOutcome::Skipped("File too large".to_owned());
    }

    let suppression = SuppressionFilter;
    let suppressed = match tool {
        ToolKind::Edit if hook_input.tool_input.edits.is_empty() => {
            suppression.is_file_suppressed(&hook_input.tool_input.new_string)
        }
        ToolKind::Edit | ToolKind::MultiEdit => hook_input
            .tool_input
            .edits
            .iter()
            .any(|edit| suppression.is_file_suppressed(&edit.new_string)),
        ToolKind::Write => suppression.is_file_suppressed(get_content_to_check(hook_input)),
    };
    if suppressed {
        return HookOutcome::Skipped("Suppressed by comment-checker: allow".to_owned());
    }

    let comments = match tool {
        ToolKind::Edit if hook_input.tool_input.edits.is_empty() => {
            if hook_input.tool_input.new_string.is_empty() {
                return HookOutcome::Skipped("No content to check".to_owned());
            }
            CommentDetector::detect_edit_checked(
                &hook_input.tool_input.old_string,
                &hook_input.tool_input.new_string,
                file_path,
                include_docstrings,
            )
        }
        ToolKind::Edit | ToolKind::MultiEdit => {
            if hook_input.tool_input.edits.is_empty() {
                return HookOutcome::Skipped("No content to check".to_owned());
            }
            edits_comments(&hook_input.tool_input.edits, file_path, include_docstrings)
        }
        ToolKind::Write => {
            let content = get_content_to_check(hook_input);
            if content.is_empty() {
                return HookOutcome::Skipped("No content to check".to_owned());
            }
            CommentDetector::detect_checked(content, file_path, include_docstrings)
        }
    };
    let comments = match comments {
        Ok(comments) => comments,
        Err(DetectError::BudgetExceeded) => {
            return HookOutcome::Skipped("Parse budget exceeded".to_owned());
        }
        Err(DetectError::Internal(reason)) => return HookOutcome::Failed(reason),
    };

    if comments.is_empty() {
        return HookOutcome::Clean;
    }

    let mut filtered = apply_filters(&comments);
    if filtered.is_empty() {
        return HookOutcome::Clean;
    }

    // A whole-file Write gives trustworthy neighbours; an Edit/MultiEdit fragment does not.
    if tool == ToolKind::Write {
        crate::filters::contextualize(&mut filtered, get_content_to_check(hook_input), true);
    }

    HookOutcome::Findings(format_hook_message(&filtered, custom_prompt))
}

/// Run the removal pass over the target file.
///
/// When the file is on disk the run analyzes its full content, because the payload names the file
/// but a partial edit fragment is not something a removal pass can rewrite safely. A missing file
/// falls back to the payload analysis and is never created.
fn evaluate_strip(
    hook_input: &HookInput,
    custom_prompt: &str,
    include_docstrings: bool,
) -> HookCheckResult {
    if classify_tool(&hook_input.tool_name).is_none() {
        return skip("Unsupported tool");
    }
    let file_path = hook_input.tool_input.file_path.as_str();
    if file_path.is_empty() {
        return skip("No file path provided");
    }
    if language_for_path(file_path).is_none() {
        return skip("Non-code file");
    }

    let target = resolve_target_path(file_path, &hook_input.cwd);
    let content = match std::fs::read_to_string(&target) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let outcome = evaluate(hook_input, custom_prompt, include_docstrings);
            return strip_message(
                &outcome,
                &format!("not rewritten - {} is not on disk", target.display()),
            );
        }
        Err(error) => {
            return strip_message(
                &HookOutcome::Failed(format!("could not read {}: {error}", target.display())),
                "not rewritten - the target could not be read",
            );
        }
    };

    if content.len() > MAX_CONTENT_BYTES {
        return strip_message(
            &HookOutcome::Skipped("File too large".to_owned()),
            "not rewritten - the file is over the inspect limit",
        );
    }
    if SuppressionFilter.is_file_suppressed(&content) {
        return strip_message(
            &HookOutcome::Skipped("Suppressed by comment-checker: allow".to_owned()),
            &format!("{} left untouched - suppressed", target.display()),
        );
    }

    let comments = match CommentDetector::detect_checked(&content, file_path, include_docstrings) {
        Ok(comments) => comments,
        Err(DetectError::BudgetExceeded) => {
            return strip_message(
                &HookOutcome::Skipped("Parse budget exceeded".to_owned()),
                "not rewritten - parse budget exceeded",
            );
        }
        Err(DetectError::Internal(reason)) => {
            return strip_message(
                &HookOutcome::Failed(reason),
                "not rewritten - the check did not run",
            );
        }
    };

    let mut filtered = apply_filters(&comments);
    if filtered.is_empty() {
        return strip_message(
            &HookOutcome::Clean,
            &format!("no flagged comments in {}", target.display()),
        );
    }
    crate::filters::contextualize(&mut filtered, &content, true);
    let report = format_hook_message(&filtered, custom_prompt);

    match strip_content(&content, &filtered) {
        Ok(stripped) => match write_atomic(&target, stripped.as_bytes()) {
            Ok(()) => strip_message(
                &HookOutcome::Findings(report),
                &format!(
                    "removed {} flagged comment(s) from {}",
                    filtered.len(),
                    target.display()
                ),
            ),
            Err(error) => strip_message(
                &HookOutcome::Findings(report),
                &format!(
                    "{} left untouched - write failed: {error}",
                    target.display()
                ),
            ),
        },
        Err(reason) => strip_message(
            &HookOutcome::Findings(report),
            &format!("{} left untouched - {reason}", target.display()),
        ),
    }
}

/// Append the strip summary to the report the flag's run would have produced anyway.
fn strip_message(outcome: &HookOutcome, summary: &str) -> HookCheckResult {
    let mut message = outcome.message();
    if !message.ends_with('\n') {
        message.push('\n');
    }
    message.push('[');
    message.push_str(env!("CARGO_PKG_NAME"));
    message.push_str("] strip: ");
    message.push_str(summary);
    message.push('\n');
    HookCheckResult {
        exit_code: outcome.exit_code(),
        message,
    }
}

fn resolve_target_path(file_path: &str, payload_cwd: &str) -> PathBuf {
    let path = Path::new(file_path);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let base = if payload_cwd.is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        PathBuf::from(payload_cwd)
    };
    base.join(path)
}

fn line_starts(content: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

fn row_of(starts: &[usize], offset: usize) -> usize {
    starts
        .partition_point(|&start| start <= offset)
        .saturating_sub(1)
}

fn line_end(starts: &[usize], len: usize, row: usize) -> usize {
    starts.get(row + 1).map_or(len, |&next| next - 1)
}

const fn is_indentation(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\r')
}

/// The byte range of one flagged comment in `content`, or why it cannot be found unambiguously.
fn locate_comment(
    content: &str,
    starts: &[usize],
    comment: &CommentInfo,
) -> Result<(usize, usize), String> {
    let row = comment.line_number.saturating_sub(1);
    if let Some((start, end)) = comment.byte_span {
        if start <= end
            && end <= content.len()
            && content.get(start..end) == Some(comment.text.as_str())
            && row_of(starts, start) == row
        {
            return Ok((start, end));
        }
    }

    let Some(&start_of_line) = starts.get(row) else {
        return Err(format!(
            "flagged comment at line {} is past the end of the file",
            comment.line_number
        ));
    };
    let end_of_line = line_end(starts, content.len(), row);
    let line = &content[start_of_line..end_of_line];
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let candidate = start_of_line + indent;
    if content[candidate..].starts_with(&comment.text) {
        return Ok((candidate, candidate + comment.text.len()));
    }

    let mut matches = line
        .match_indices(&comment.text)
        .map(|(index, _)| start_of_line + index);
    let found = (matches.next(), matches.next());
    match found {
        (Some(start), None) if content[start..].starts_with(&comment.text) => {
            Ok((start, start + comment.text.len()))
        }
        (Some(_), Some(_)) => Err(format!(
            "flagged comment at line {} matches more than once",
            comment.line_number
        )),
        _ => Err(format!(
            "flagged comment at line {} could not be located",
            comment.line_number
        )),
    }
}

/// The bytes one comment's removal deletes. A comment that occupies its own line takes the line
/// including its ending; a trailing comment keeps the code and the ending; anything else takes just
/// its span.
fn removal_range(
    content: &str,
    starts: &[usize],
    start: usize,
    end: usize,
) -> Result<(usize, usize), String> {
    if end <= start || end > content.len() {
        return Err("flagged comment has an empty span".to_owned());
    }
    let start_row = row_of(starts, start);
    let end_row = row_of(starts, end - 1);
    let line_start = starts[start_row];
    let before = &content[line_start..start];
    let after = &content[end..line_end(starts, content.len(), end_row)];
    let owns_line = before.chars().all(is_indentation) && after.chars().all(is_indentation);
    if owns_line {
        let line_end = starts.get(end_row + 1).copied().unwrap_or(content.len());
        return Ok((line_start, line_end));
    }
    if before
        .chars()
        .any(|character| !matches!(character, ' ' | '\t'))
    {
        let trimmed = before.trim_end_matches([' ', '\t']).len();
        return Ok((line_start + trimmed, end));
    }
    Ok((start, end))
}

fn strip_content(content: &str, comments: &[CommentInfo]) -> Result<String, String> {
    let starts = line_starts(content);
    let mut spans = Vec::with_capacity(comments.len());
    for comment in comments {
        spans.push(locate_comment(content, &starts, comment)?);
    }
    spans.sort_unstable();

    let mut located: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(&last) = located.last() {
            if span == last {
                continue;
            }
            if span.0 < last.1 {
                return Err(format!(
                    "flagged comments overlap near line {}",
                    row_of(&starts, span.0) + 1
                ));
            }
        }
        located.push(span);
    }

    let mut edits = Vec::with_capacity(located.len());
    for (start, end) in located {
        edits.push(removal_range(content, &starts, start, end)?);
    }
    edits.sort_unstable();
    for pair in edits.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err("flagged removals overlap".to_owned());
        }
    }

    let mut output = String::with_capacity(content.len());
    let mut output_starts = vec![0];
    let mut output_origins = vec![0];
    let mut cursor = 0;
    for (start, end) in edits {
        if start < cursor {
            return Err("flagged removals overlap".to_owned());
        }
        append_segment(
            content,
            cursor,
            start,
            &mut output,
            &mut output_starts,
            &mut output_origins,
        );
        cursor = end;
    }
    append_segment(
        content,
        cursor,
        content.len(),
        &mut output,
        &mut output_starts,
        &mut output_origins,
    );
    drop_emptied_lines(
        content,
        &starts,
        &mut output,
        &output_starts,
        &output_origins,
    );
    Ok(output)
}

/// Copy `content[from..to]` and remember which original byte starts each new output line, so a line
/// emptied by a removal can be told apart from a blank line that was already there.
fn append_segment(
    content: &str,
    from: usize,
    to: usize,
    output: &mut String,
    starts: &mut Vec<usize>,
    origins: &mut Vec<usize>,
) {
    if from >= to {
        return;
    }
    if output.len() == *starts.last().unwrap_or(&0) {
        if let Some(origin) = origins.last_mut() {
            *origin = from;
        }
    }
    let base = output.len();
    for (offset, byte) in content[from..to].bytes().enumerate() {
        if byte == b'\n' {
            starts.push(base + offset + 1);
            origins.push(from + offset + 1);
        }
    }
    output.push_str(&content[from..to]);
}

fn is_blank(line: &str) -> bool {
    line.chars()
        .all(|character| matches!(character, ' ' | '\t' | '\r'))
}

/// Drop each output line that a removal left blank, keeping blank lines the file already had.
fn drop_emptied_lines(
    content: &str,
    original_starts: &[usize],
    output: &mut String,
    starts: &[usize],
    origins: &[usize],
) {
    let mut keep = vec![true; starts.len()];
    let mut changed = false;
    for (index, &start) in starts.iter().enumerate() {
        if start >= output.len() {
            continue;
        }
        let end = starts.get(index + 1).map_or(output.len(), |&next| next - 1);
        if !is_blank(&output[start..end]) {
            continue;
        }
        let origin = origins.get(index).copied().unwrap_or(0);
        if origin >= content.len() {
            continue;
        }
        let original_row = row_of(original_starts, origin);
        let original_start = original_starts[original_row];
        let original_end = line_end(original_starts, content.len(), original_row);
        if !is_blank(&content[original_start..original_end]) {
            keep[index] = false;
            changed = true;
        }
    }
    if !changed {
        return;
    }
    let mut rebuilt = String::with_capacity(output.len());
    for (index, &start) in starts.iter().enumerate() {
        if keep[index] {
            let end = starts.get(index + 1).copied().unwrap_or(output.len());
            rebuilt.push_str(&output[start..end]);
        }
    }
    *output = rebuilt;
}

/// Replace the target in one pass, keeping the old bytes intact until the new file is complete.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| Path::new("."), |parent| parent);
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("target has no file name"))?;
    let temporary = parent.join(format!(
        ".{}.{}.strip.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = std::fs::write(&temporary, bytes).and_then(|()| {
        if let Some(permissions) = permissions {
            std::fs::set_permissions(&temporary, permissions)?;
        }
        std::fs::rename(&temporary, path)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Whether any content this call would inspect is over [`MAX_CONTENT_BYTES`].
fn exceeds_content_limit(tool: ToolKind, hook_input: &HookInput) -> bool {
    match tool {
        ToolKind::Edit if hook_input.tool_input.edits.is_empty() => {
            hook_input.tool_input.new_string.len() > MAX_CONTENT_BYTES
        }
        ToolKind::Edit | ToolKind::MultiEdit => hook_input
            .tool_input
            .edits
            .iter()
            .any(|edit| edit.new_string.len() > MAX_CONTENT_BYTES),
        ToolKind::Write => get_content_to_check(hook_input).len() > MAX_CONTENT_BYTES,
    }
}

/// Comments for a list of discrete edits: a `MultiEdit`, or an `Edit`/`apply_patch` that attached an
/// `edits[]` array (the `OpenCode` plugin's shape).
fn edits_comments(
    edits: &[Edit],
    file_path: &str,
    include_docstrings: bool,
) -> Result<Vec<CommentInfo>, DetectError> {
    let mut comments = Vec::new();
    for edit in edits {
        if edit.new_string.is_empty() {
            continue;
        }
        comments.extend(CommentDetector::detect_edit_checked(
            &edit.old_string,
            &edit.new_string,
            file_path,
            include_docstrings,
        )?);
    }
    Ok(comments)
}

#[must_use]
pub fn check_hook_input_json(input: &str, custom_prompt: &str) -> String {
    engine::result_json(&check_hook_input(input, custom_prompt))
}

#[must_use]
pub fn detect_comments_json(content: &str, file_path: &str, include_docstrings: bool) -> String {
    match CommentDetector::detect_checked(content, file_path, include_docstrings) {
        Ok(comments) => serde_json::to_string(&comments)
            .unwrap_or_else(|error| engine::fallback_json(&error.to_string())),
        Err(error) => engine::result_json(&HookOutcome::Failed(error.to_string()).into_result()),
    }
}

fn get_content_to_check(input: &HookInput) -> &str {
    if input.tool_input.content.is_empty() {
        input.tool_input.new_string.as_str()
    } else {
        input.tool_input.content.as_str()
    }
}

fn skip(reason: &str) -> HookCheckResult {
    HookOutcome::Skipped(reason.to_owned()).into_result()
}

#[cfg(test)]
mod tests {
    use super::check_hook_input;
    use crate::detector::{
        FORCED_INTERNAL_FAILURE_REASON, set_forced_budget_exceeded, set_forced_internal_failure,
    };

    /// Both seams are process-wide switches; serialize the tests that flip them so one test never
    /// observes the other's switch.
    static TEST_SEAM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_seams() -> std::sync::MutexGuard<'static, ()> {
        TEST_SEAM_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Clears the test-only switch even if an assertion unwinds.
    struct ForcedInternalFailure;

    impl ForcedInternalFailure {
        fn enable() -> Self {
            set_forced_internal_failure(true);
            Self
        }
    }

    impl Drop for ForcedInternalFailure {
        fn drop(&mut self) {
            set_forced_internal_failure(false);
        }
    }

    /// Clears the test-only budget switch even if an assertion unwinds.
    struct ForcedBudgetExceeded;

    impl ForcedBudgetExceeded {
        fn enable() -> Self {
            set_forced_budget_exceeded(true);
            Self
        }
    }

    impl Drop for ForcedBudgetExceeded {
        fn drop(&mut self) {
            set_forced_budget_exceeded(false);
        }
    }

    #[test]
    fn internal_failure_exits_one_and_reports_did_not_run() {
        let _seams = lock_seams();
        let _forced = ForcedInternalFailure::enable();
        let input = r##"{"tool_name":"Write","tool_input":{"file_path":"test.py","content":"# comment\nprint(1)"}}"##;

        let result = check_hook_input(input, "");

        assert_eq!(result.exit_code, 1);
        assert_eq!(
            result.message,
            format!(
                "[check-comments] did not run — nothing checked this write.\n{FORCED_INTERNAL_FAILURE_REASON}\n"
            )
        );
    }

    #[test]
    fn budget_exceeded_exits_zero_with_a_visible_skip_not_a_failure() {
        let _seams = lock_seams();
        let _forced = ForcedBudgetExceeded::enable();
        let input = r##"{"tool_name":"Write","tool_input":{"file_path":"test.py","content":"# comment\nprint(1)"}}"##;

        let result = check_hook_input(input, "");

        assert_eq!(result.exit_code, 0);
        assert_eq!(
            result.message,
            "[check-comments] Skipping: Parse budget exceeded\n"
        );
        assert!(
            !result.message.contains("did not run"),
            "message: {}",
            result.message
        );
    }
}

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
        |hook_input| evaluate(&hook_input, &options.prompt, true).into_result(),
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

use serde::{Deserialize, Serialize};

use crate::detector::CommentDetector;
use crate::engine::{self, null_default};
use crate::filters::{SuppressionFilter, apply_filters};
use crate::language::language_for_path;
use crate::models::Edit;
use crate::output::format_hook_message;

pub const EXIT_PASS: i32 = 0;
pub const EXIT_BLOCK: i32 = 2;

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
        |hook_input| evaluate(&hook_input, custom_prompt, include_docstrings),
    )
}

#[must_use]
pub fn check_hook_input_from_reader<R: std::io::Read>(
    reader: R,
    options: &crate::engine::Options,
) -> HookCheckResult {
    serde_json::from_reader::<R, HookInput>(reader).map_or_else(
        |_| skip("Invalid input format"),
        |hook_input| evaluate(&hook_input, &options.prompt, true),
    )
}

fn evaluate(
    hook_input: &HookInput,
    custom_prompt: &str,
    include_docstrings: bool,
) -> HookCheckResult {
    let file_path = hook_input.tool_input.file_path.as_str();
    if file_path.is_empty() {
        return skip("No file path provided");
    }
    if language_for_path(file_path).is_none() {
        return skip("Non-code file");
    }

    let suppression = SuppressionFilter;
    let suppressed = match hook_input.tool_name.as_str() {
        "Edit" => suppression.is_file_suppressed(&hook_input.tool_input.new_string),
        "MultiEdit" => hook_input
            .tool_input
            .edits
            .iter()
            .any(|edit| suppression.is_file_suppressed(&edit.new_string)),
        _ => suppression.is_file_suppressed(get_content_to_check(hook_input)),
    };
    if suppressed {
        return skip("Suppressed by comment-checker: allow");
    }

    let detector = CommentDetector::new();
    let comments = match hook_input.tool_name.as_str() {
        "Edit" => {
            if hook_input.tool_input.new_string.is_empty() {
                return skip("No content to check");
            }
            detector.detect_edit(
                &hook_input.tool_input.old_string,
                &hook_input.tool_input.new_string,
                file_path,
                include_docstrings,
            )
        }
        "MultiEdit" => {
            if hook_input.tool_input.edits.is_empty() {
                return skip("No content to check");
            }
            let mut comments = Vec::new();
            for edit in &hook_input.tool_input.edits {
                if edit.new_string.is_empty() {
                    continue;
                }
                comments.extend(detector.detect_edit(
                    &edit.old_string,
                    &edit.new_string,
                    file_path,
                    include_docstrings,
                ));
            }
            comments
        }
        _ => {
            let content = get_content_to_check(hook_input);
            if content.is_empty() {
                return skip("No content to check");
            }
            detector.detect(content, file_path, include_docstrings)
        }
    };

    if comments.is_empty() {
        return success();
    }

    let filtered = apply_filters(&comments);
    if filtered.is_empty() {
        return success();
    }

    HookCheckResult::block(format_hook_message(&filtered, custom_prompt))
}

#[must_use]
pub fn check_hook_input_json(input: &str, custom_prompt: &str) -> String {
    engine::result_json(&check_hook_input(input, custom_prompt))
}

#[must_use]
pub fn detect_comments_json(content: &str, file_path: &str, include_docstrings: bool) -> String {
    let detector = CommentDetector::new();
    let comments = detector.detect(content, file_path, include_docstrings);
    match serde_json::to_string(&comments) {
        Ok(json) => json,
        Err(error) => engine::fallback_json(&error.to_string()),
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
    HookCheckResult::pass(format!("[check-comments] Skipping: {reason}\n"))
}

fn success() -> HookCheckResult {
    HookCheckResult::pass("[check-comments] Success: No problematic comments/docstrings found\n")
}

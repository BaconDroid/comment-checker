use serde::{Deserialize, Deserializer, Serialize};

use crate::detector::CommentDetector;
use crate::filters::apply_filters;
use crate::language::language_for_path;
use crate::models::CommentInfo;
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
    edits: Vec<EditInput>,
}

#[derive(Debug, Default, Deserialize)]
struct EditInput {
    #[serde(default, alias = "oldString", deserialize_with = "null_default")]
    old_string: String,
    #[serde(default, alias = "newString", deserialize_with = "null_default")]
    new_string: String,
}

#[derive(Debug, Deserialize)]
struct HookInput {
    #[serde(default, alias = "toolName", deserialize_with = "null_default")]
    tool_name: String,
    #[serde(default, alias = "toolInput", deserialize_with = "null_default")]
    tool_input: ToolInput,
}

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[must_use]
pub fn check_hook_input(input: &str, custom_prompt: &str) -> HookCheckResult {
    if input.is_empty() {
        return skip("No input provided");
    }

    serde_json::from_str::<HookInput>(input).map_or_else(
        |_| skip("Invalid input format"),
        |hook_input| evaluate(&hook_input, custom_prompt),
    )
}

#[must_use]
pub fn check_hook_input_from_reader<R: std::io::Read>(
    reader: R,
    custom_prompt: &str,
) -> HookCheckResult {
    serde_json::from_reader::<R, HookInput>(reader).map_or_else(
        |_| skip("Invalid input format"),
        |hook_input| evaluate(&hook_input, custom_prompt),
    )
}

fn evaluate(hook_input: &HookInput, custom_prompt: &str) -> HookCheckResult {
    let file_path = hook_input.tool_input.file_path.as_str();
    if file_path.is_empty() {
        return skip("No file path provided");
    }
    if language_for_path(file_path).is_none() {
        return skip("Non-code file");
    }

    let detector = CommentDetector::new();
    let comments = match hook_input.tool_name.as_str() {
        "Edit" => {
            if hook_input.tool_input.new_string.is_empty() {
                return skip("No content to check");
            }
            detect_new_comments_for_edit(
                &detector,
                &hook_input.tool_input.old_string,
                &hook_input.tool_input.new_string,
                file_path,
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
                comments.extend(detect_new_comments_for_edit(
                    &detector,
                    &edit.old_string,
                    &edit.new_string,
                    file_path,
                ));
            }
            comments
        }
        _ => {
            let content = get_content_to_check(hook_input);
            if content.is_empty() {
                return skip("No content to check");
            }
            detector.detect(content, file_path, true)
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
    result_json(&check_hook_input(input, custom_prompt))
}

#[must_use]
pub fn detect_comments_json(content: &str, file_path: &str, include_docstrings: bool) -> String {
    let detector = CommentDetector::new();
    let comments = detector.detect(content, file_path, include_docstrings);
    match serde_json::to_string(&comments) {
        Ok(json) => json,
        Err(error) => fallback_json(&error.to_string()),
    }
}

fn get_content_to_check(input: &HookInput) -> &str {
    if input.tool_input.content.is_empty() {
        input.tool_input.new_string.as_str()
    } else {
        input.tool_input.content.as_str()
    }
}

fn detect_new_comments_for_edit(
    detector: &CommentDetector,
    old_string: &str,
    new_string: &str,
    file_path: &str,
) -> Vec<CommentInfo> {
    let old_comments = detector.detect(old_string, file_path, true);
    let new_comments = detector.detect(new_string, file_path, true);
    filter_new_comments(&old_comments, &new_comments)
}

fn filter_new_comments(
    old_comments: &[CommentInfo],
    new_comments: &[CommentInfo],
) -> Vec<CommentInfo> {
    if old_comments.is_empty() {
        return new_comments.to_vec();
    }
    let old_texts: std::collections::HashSet<String> = old_comments
        .iter()
        .map(CommentInfo::normalized_text)
        .collect();
    new_comments
        .iter()
        .filter(|comment| {
            let normalized = comment.normalized_text();
            !old_texts.contains(normalized.as_str())
        })
        .cloned()
        .collect()
}

fn skip(reason: &str) -> HookCheckResult {
    HookCheckResult::pass(format!("[check-comments] Skipping: {reason}\n"))
}

fn success() -> HookCheckResult {
    HookCheckResult::pass("[check-comments] Success: No problematic comments/docstrings found\n")
}

fn result_json(result: &HookCheckResult) -> String {
    match serde_json::to_string(result) {
        Ok(json) => json,
        Err(error) => fallback_json(&error.to_string()),
    }
}

fn fallback_json(error: &str) -> String {
    serde_json::json!({
        "exitCode": 1,
        "message": format!("serialization failed: {error}"),
    })
    .to_string()
}

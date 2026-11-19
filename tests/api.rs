use comment_checker::api::{
    check_hook_input, check_hook_input_json, check_hook_input_with_options, detect_comments_json,
};
use serde_json::Value;

#[test]
fn check_hook_input_given_clean_write_returns_pass() {
    // given
    let input =
        r#"{"tool_name":"Write","tool_input":{"file_path":"test.py","content":"print(1)"}}"#;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 0);
    assert!(result.message.contains("Success"));
}

#[test]
fn check_hook_input_json_given_comment_returns_camel_case_block() -> serde_json::Result<()> {
    // given
    let input = r##"{"tool_name":"Write","tool_input":{"file_path":"test.py","content":"# comment\nprint(1)"}}"##;

    // when
    let json: Value = serde_json::from_str(&check_hook_input_json(input, ""))?;

    // then
    assert_eq!(json.get("exitCode").and_then(Value::as_i64), Some(2));
    assert!(
        json.get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("<comments file=\"test.py\">"))
    );
    Ok(())
}

#[test]
fn detect_comments_json_given_python_comment_returns_comment_payload() -> serde_json::Result<()> {
    // given
    let content = "# comment\nprint(1)";

    // when
    let json: Value = serde_json::from_str(&detect_comments_json(content, "test.py", true))?;

    // then
    assert_eq!(json.as_array().map(Vec::len), Some(1));
    assert_eq!(
        json.as_array()
            .and_then(|comments| comments.first())
            .and_then(|comment| comment.get("lineNumber"))
            .and_then(Value::as_u64),
        Some(1),
    );
    Ok(())
}

#[test]
fn check_hook_input_given_camel_case_keys_and_null_fields_parses() {
    // given
    let input = r##"{"toolName":"Write","toolInput":{"filePath":"test.py","content":"# comment\nprint(1)","edits":null}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
}

#[test]
fn check_hook_input_with_options_given_docstrings_disabled_returns_pass() {
    // given
    let input = r#"{"tool_name":"Write","tool_input":{"file_path":"test.py","content":"\"\"\"doc\"\"\"\nprint(1)"}}"#;

    // when
    let result = check_hook_input_with_options(input, "", false);

    // then
    assert_eq!(result.exit_code, 0);
}

#[test]
fn check_hook_input_with_options_given_docstrings_enabled_exits_two() {
    // given
    let input = r#"{"tool_name":"Write","tool_input":{"file_path":"test.py","content":"\"\"\"doc\"\"\"\nprint(1)"}}"#;

    // when
    let result = check_hook_input_with_options(input, "", true);

    // then
    assert_eq!(result.exit_code, 2);
}

#[test]
fn check_hook_input_given_edit_adding_comment_with_repeated_text_exits_two() {
    // given: the added `# note` is text that already existed later in old_string
    let input = r##"{"tool_name":"Edit","tool_input":{"file_path":"test.py","old_string":"# note\nx = 1\n","new_string":"# note\nx = 1\n# note\n"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
}

#[test]
fn check_hook_input_given_file_suppression_marker_skips() {
    // given
    let input = r##"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"# comment-checker: allow\n# comment\nprint(1)"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 0);
    assert!(
        result
            .message
            .contains("Suppressed by comment-checker: allow")
    );
}

#[test]
fn check_hook_input_given_marker_inside_a_string_literal_does_not_suppress() {
    // given: the marker text is a Python string literal, not a comment
    let input = r#"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"x = \"comment-checker: allow\"\n# calculate the sum\n"}}"#;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        result.message.contains("reason=\"unnecessary-comment\""),
        "message: {}",
        result.message
    );
}

#[test]
fn check_hook_input_given_marker_with_a_spared_class_and_justification_suppresses() {
    // given
    let input = r##"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"# comment-checker: allow bdd -- the fixture waits for a real server\n# calculate the sum\n"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 0);
    assert!(
        result
            .message
            .contains("Suppressed by comment-checker: allow"),
        "message: {}",
        result.message
    );
}

#[test]
fn check_hook_input_given_marker_added_by_the_edit_reports_allow_added() {
    // given: the same edit introduces the marker and the code it would suppress
    let input = r#"{"tool_name":"Edit","tool_input":{"file_path":"a.py","old_string":"print(1)\n","new_string":"print(1)\n# comment-checker: allow\n"}}"#;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        result.message.contains("reason=\"allow-added\""),
        "message: {}",
        result.message
    );
}

#[test]
fn check_hook_input_given_marker_with_an_unknown_class_reports_allow_invalid() {
    // given: `not-a-class` is not one of the spared class ids
    let input = r##"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"# comment-checker: allow not-a-class -- because\n# calculate the sum\n"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        result.message.contains("reason=\"allow-invalid\""),
        "message: {}",
        result.message
    );
    assert!(
        !result.message.contains("Suppressed"),
        "message: {}",
        result.message
    );
}

#[test]
fn check_hook_input_given_marker_with_an_empty_justification_reports_allow_invalid() {
    // given
    let input = r##"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"# comment-checker: allow bdd --\n# calculate the sum\n"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        result.message.contains("reason=\"allow-invalid\""),
        "message: {}",
        result.message
    );
}

#[test]
fn check_hook_input_given_unknown_tool_skips() {
    // given: a non-edit tool, which the hook matcher (Write|Edit|MultiEdit) never sends
    let input = r##"{"tool_name":"Read","tool_input":{"file_path":"test.py","content":"# comment\nprint(1)"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 0);
}

#[test]
fn check_hook_input_given_apply_patch_adding_comment_exits_two() {
    // given: the OpenCode plugin forwards `apply_patch`, which must be checked as an edit.
    let input = r#"{"tool_name":"apply_patch","tool_input":{"file_path":"a.py","old_string":"print(1)\n","new_string":"print(1)\n# comment\n"}}"#;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
}

#[test]
fn check_hook_input_given_lowercase_write_exits_two() {
    // given: the OpenCode plugin forwards the tool name lowercased.
    let input = r##"{"tool_name":"write","tool_input":{"file_path":"a.py","content":"# comment\nprint(1)"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
}

#[test]
fn check_hook_input_given_edit_with_edits_array_analyzes_the_edits() {
    // given: an `Edit` carrying the plugin's `edits[]` array, with the top-level `new_string` empty.
    let input = r#"{"tool_name":"Edit","tool_input":{"file_path":"a.py","edits":[{"old_string":"print(1)\n","new_string":"print(1)\n# comment\n"}]}}"#;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
}

#[test]
fn check_hook_input_given_mapped_but_unshipped_grammar_skips_visibly() {
    // given: the pack maps these paths, but this binary compiles no grammar for them
    let cases = [
        ("README.md", "# heading\n"),
        ("config.json", "{\"a\": 1}\n"),
        ("Makefile", "all:\n\techo hi\n"),
        ("setup.ini", "[a]\nb = 1\n"),
    ];

    for (file_path, content) in cases {
        let input = serde_json::json!({
            "tool_name": "Write",
            "tool_input": { "file_path": file_path, "content": content },
        })
        .to_string();

        // when
        let result = check_hook_input(&input, "");

        // then: a visible skip, never "did not run"
        assert_eq!(result.exit_code, 0, "path: {file_path}");
        assert_eq!(
            result.message, "[check-comments] Skipping: Non-code file\n",
            "path: {file_path}"
        );
    }
}

#[test]
fn check_hook_input_given_content_over_the_limit_skips_visibly() {
    // given: a write larger than the documented 2 MiB inspection limit.
    let content = "x".repeat(2 * 1024 * 1024 + 1);
    let input = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": "a.py", "content": content },
    })
    .to_string();

    // when
    let result = check_hook_input(&input, "");

    // then: a skip, never a pass message
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.message,
        "[check-comments] Skipping: File too large\n"
    );
    assert!(!result.message.contains("Success"));
}

/// Dense JavaScript whose parse work grows with `target_bytes`. At ~150 KB it needs ~3,000 progress
/// ticks (below the 5,000-tick operation budget); at ~300 KB it needs ~5,900 (above it).
fn dense_javascript(target_bytes: usize) -> String {
    use std::fmt::Write as _;

    let mut content = String::with_capacity(target_bytes + 128);
    let mut index = 0;
    while content.len() < target_bytes {
        let _ = writeln!(
            content,
            "it(\"case {index}\", () => {{ expect(outer(inner({index}))).toBe({index}); }});"
        );
        index += 1;
    }
    content.push_str("// plain comment\n");
    content
}

#[test]
fn check_hook_input_given_parse_over_operation_budget_skips_deterministically() {
    // given: dense JavaScript above the operation budget, whose parse outcome used to depend on
    // whether the shared wall clock happened to abort first
    let content = dense_javascript(300_000);
    let payload = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": "tests/bulk.js", "content": content },
    })
    .to_string();

    // when: identical bytes go through the hook repeatedly
    let results: Vec<_> = (0..5).map(|_| check_hook_input(&payload, "")).collect();

    // then: every run reports the same visible skip, never a finding and never a clean pass
    for result in &results {
        assert_eq!(result.exit_code, 0);
        assert_eq!(
            result.message,
            "[check-comments] Skipping: Parse budget exceeded\n"
        );
    }
}

#[test]
fn check_hook_input_given_parse_under_operation_budget_analyzes_deterministically() {
    // given: dense JavaScript of the same shape but below the operation budget
    let content = dense_javascript(150_000);
    let payload = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": "tests/bulk.js", "content": content },
    })
    .to_string();

    // when: identical bytes go through the hook repeatedly
    let results: Vec<_> = (0..5).map(|_| check_hook_input(&payload, "")).collect();

    // then: the file is analyzed every time with the same complete result
    let first = &results[0];
    assert_eq!(first.exit_code, 2, "{}", first.message);
    assert!(first.message.contains("plain comment"), "{}", first.message);
    for result in &results[1..] {
        assert_eq!(result.exit_code, first.exit_code);
        assert_eq!(result.message, first.message);
    }
}

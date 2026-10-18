use comment_checker::detector::CommentDetector;
use comment_checker::filters::{AgentMemoFilter, apply_filters};
use comment_checker::output::format_hook_message;

#[test]
fn full_pipeline_given_no_comments_returns_empty() {
    // given
    let detector = CommentDetector::new();
    let code = "package main\n\nfunc main() {\n\tprintln(\"hello\")\n}";

    // when
    let comments = detector.detect(code, "main.go", false);

    // then
    assert_eq!(comments.len(), 0);
}

#[test]
fn full_pipeline_given_bdd_comment_filters_out() {
    // given
    let detector = CommentDetector::new();
    let code = "# given\ndef test_something():\n    pass";

    // when
    let comments = detector.detect(code, "test.py", false);
    let filtered = apply_filters(&comments);

    // then
    assert_eq!(comments.len(), 1);
    assert_eq!(filtered.len(), 0);
}

#[test]
fn full_pipeline_given_regular_comment_returns_formatted_message() {
    // given
    let detector = CommentDetector::new();
    let code = "# This is a regular comment\nprint(\"hello\")";

    // when
    let comments = detector.detect(code, "test.py", false);
    let filtered = apply_filters(&comments);
    let message = format_hook_message(&filtered, "");

    // then
    assert_eq!(filtered.len(), 1);
    assert!(message.contains("COMMENT/DOCSTRING DETECTED"));
    assert!(message.contains("test.py"));
    assert!(message.contains("This is a regular comment"));
}

#[test]
fn full_pipeline_given_docstring_keeps_as_code_smell() {
    // given
    let detector = CommentDetector::new();
    let code = "\"\"\"This is a docstring\"\"\"\ndef hello():\n    pass";

    // when
    let comments = detector.detect(code, "test.py", true);
    let filtered = apply_filters(&comments);

    // then
    assert_eq!(comments.len(), 1);
    assert_eq!(filtered.len(), 1);
}

#[test]
fn full_pipeline_given_directive_filters_out() {
    // given
    let detector = CommentDetector::new();
    let code = "# noqa: E501\nprint(\"very long line\")";

    // when
    let comments = detector.detect(code, "test.py", false);
    let filtered = apply_filters(&comments);

    // then
    assert_eq!(comments.len(), 1);
    assert_eq!(filtered.len(), 0);
}

#[test]
fn full_pipeline_given_shebang_filters_out() {
    // given
    let detector = CommentDetector::new();
    let code = "#!/usr/bin/env python\nprint(\"hello\")";

    // when
    let comments = detector.detect(code, "test.py", false);
    let filtered = apply_filters(&comments);

    // then
    assert_eq!(comments.len(), 1);
    assert_eq!(filtered.len(), 0);
}

#[test]
fn full_pipeline_given_agent_memo_detects_code_smell() {
    // given
    let detector = CommentDetector::new();
    let agent_memo_filter = AgentMemoFilter;
    let code = "# Changed from old_value to new_value\nprint(\"hello\")";

    // when
    let comments = detector.detect(code, "test.py", false);
    let filtered = apply_filters(&comments);

    // then
    assert_eq!(filtered.len(), 1);
    assert!(agent_memo_filter.is_agent_memo(&filtered[0]));
}

#[test]
fn full_pipeline_given_agent_memo_formatter_includes_warning() {
    // given
    let detector = CommentDetector::new();
    let code = "# Modified to use new implementation\nprint(\"hello\")";

    // when
    let comments = detector.detect(code, "test.py", false);
    let filtered = apply_filters(&comments);
    let message = format_hook_message(&filtered, "");

    // then
    assert!(message.contains("AGENT MEMO"));
    assert!(message.contains("CODE SMELL"));
}

#[test]
fn full_pipeline_given_switched_from_detects_code_smell() {
    // given
    let detector = CommentDetector::new();
    let agent_memo_filter = AgentMemoFilter;
    let code = "# Switched from old to new\nprint(\"hello\")";

    // when
    let comments = detector.detect(code, "test.py", false);
    let filtered = apply_filters(&comments);

    // then
    assert_eq!(filtered.len(), 1);
    assert!(agent_memo_filter.is_agent_memo(&filtered[0]));
}

#[test]
fn full_pipeline_given_regular_comment_is_not_agent_memo() {
    // given
    let detector = CommentDetector::new();
    let agent_memo_filter = AgentMemoFilter;
    let code = "# Calculate the sum of values\nprint(\"hello\")";

    // when
    let comments = detector.detect(code, "test.py", false);
    let filtered = apply_filters(&comments);

    // then
    assert_eq!(filtered.len(), 1);
    assert!(!agent_memo_filter.is_agent_memo(&filtered[0]));
}

use comment_checker::api::check_hook_input;

#[test]
fn write_restating_comment_cites_evidence() {
    // given: a comment that restates the code beside it, in a whole-file Write
    let input = r#"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"counter += 1\n# increment the counter\n"}}"#;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        result.message.contains("reason=\"restates-code: counter\""),
        "message: {}",
        result.message
    );
}

#[test]
fn write_literal_payload_is_not_cited() {
    // given: the only overlap is inside a string literal, which must be masked before citing
    let input = r#"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"x = \"counter increment\"\n# counter increment\n"}}"#;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        !result.message.contains("restates-code"),
        "a literal payload must not be cited: {}",
        result.message
    );
}

#[test]
fn write_adjacent_code_skips_the_comment_body() {
    // given: a block comment whose continuation lines are not marker-prefixed, then the code
    let input = r#"{"tool_name":"Write","tool_input":{"file_path":"a.c","content":"/*\nincrement the counter\n*/\ncounter += 1;\n"}}"#;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        result.message.contains("reason=\"restates-code: counter\""),
        "message: {}",
        result.message
    );
}

#[test]
fn write_escaped_quote_literal_is_not_cited() {
    // given: an operator inside a string that also contains an escaped quote
    let input = r#"{"tool_name":"Write","tool_input":{"file_path":"a.js","content":"// increment it\nlog(\"a \\\" += b\");\n"}}"#;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        !result.message.contains("restates-code"),
        "an operator inside a literal must not be cited: {}",
        result.message
    );
}

#[test]
fn write_plain_comment_keeps_the_generic_reason() {
    // given: a comment with nothing in common with the code beside it
    let input = r#"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"print(1)\n# fetch the user record\n"}}"#;

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
fn write_control_flow_narration_is_named() {
    // given: a comment that names the loop the code already shows
    let input = r##"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"# loop over the items\nfor item in items:\n    pass\n"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        result.message.contains("reason=\"narrates-control-flow\""),
        "message: {}",
        result.message
    );
}

#[test]
fn write_operator_restatement_cites_the_operator() {
    // given: a comment whose verb maps to the operator the adjacent code uses
    let input = r##"{"tool_name":"Write","tool_input":{"file_path":"a.py","content":"# increment it\ncounter += 1\n"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then
    assert_eq!(result.exit_code, 2);
    assert!(
        result
            .message
            .contains("reason=\"restates-code: increment +=\""),
        "message: {}",
        result.message
    );
}

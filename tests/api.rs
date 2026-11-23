use comment_checker::api::{
    check_hook_input, check_hook_input_json, check_hook_input_with_options, detect_comments_json,
};
use serde_json::Value;
use std::fmt::Write as _;

fn git_root_fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cc-b1-api-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

#[test]
fn check_hook_input_given_config_at_git_root_exempts_the_reason_class() {
    // given: a git root whose config exempts agent-memo under tests/**
    let root = git_root_fixture("allow");
    std::fs::write(
        root.join(".comment-checker.json"),
        r#"{"allow":[{"glob":"tests/**","reasons":["agent-memo"],"reason":"legacy suite notes"}]}"#,
    )
    .unwrap();
    let input = format!(
        r##"{{"tool_name":"Write","tool_input":{{"file_path":"tests/app.py","content":"# Changed from old to new\nx = 1\n"}},"cwd":"{}"}}"##,
        root.display()
    );

    // when
    let exempted = check_hook_input(&input, "");

    // then: the covered agent-memo comment is exempt
    assert_eq!(exempted.exit_code, 0, "{}", exempted.message);

    // and: a config whose glob does not cover the path leaves it reported (teeth: the config is
    // consulted, and only a matching entry spares)
    std::fs::write(
        root.join(".comment-checker.json"),
        r#"{"allow":[{"glob":"vendor/**","reasons":["agent-memo"],"reason":"legacy suite notes"}]}"#,
    )
    .unwrap();
    let uncovered = check_hook_input(&input, "");
    assert_eq!(uncovered.exit_code, 2, "{}", uncovered.message);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn check_hook_input_given_root_config_edit_reports_config_edited_for_every_edit_tool() {
    // given: a git root whose config tries to exempt everything, including config-edited
    let root = git_root_fixture("config-edited");
    std::fs::write(
        root.join(".comment-checker.json"),
        r#"{"allow":[{"glob":"**","reasons":["config-edited","unnecessary-comment"],"reason":"exempt all"}]}"#,
    )
    .unwrap();
    let cwd = root.display();
    let tool_inputs = [
        (
            "Write",
            r#"{"file_path":".comment-checker.json","content":"{}"}"#,
        ),
        (
            "Edit",
            r#"{"file_path":".comment-checker.json","old_string":"exempt all","new_string":"x"}"#,
        ),
        (
            "MultiEdit",
            r#"{"file_path":".comment-checker.json","edits":[{"old_string":"a","new_string":"b"}]}"#,
        ),
    ];
    for (tool, tool_input) in tool_inputs {
        let input = format!(r#"{{"tool_name":"{tool}","tool_input":{tool_input},"cwd":"{cwd}"}}"#);
        // when
        let result = check_hook_input(&input, "");
        // then: reported and blocked even though no comment is present and the config exempts it
        assert_eq!(result.exit_code, 2, "{tool}: {}", result.message);
        assert!(
            result.message.contains(r#"reason="config-edited""#),
            "{tool}: {}",
            result.message
        );
        assert!(
            result
                .message
                .contains(r#"<comments file=".comment-checker.json">"#),
            "{tool}: {}",
            result.message
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}
#[test]
fn check_hook_input_given_config_path_spelling_resolves_to_the_root_config_only() {
    // given: a git root with a nested directory, a config beside it, and a directory outside
    let root = git_root_fixture("config-paths");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let outside = git_root_fixture("config-outside");
    let spelling_cases = [
        (".comment-checker.json".to_owned(), 2),
        ("./.comment-checker.json".to_owned(), 2),
        ("sub/../.comment-checker.json".to_owned(), 2),
        (root.join(".comment-checker.json").display().to_string(), 2),
        ("sub/.comment-checker.json".to_owned(), 0),
        (
            root.join("sub")
                .join(".comment-checker.json")
                .display()
                .to_string(),
            0,
        ),
        (
            outside.join(".comment-checker.json").display().to_string(),
            0,
        ),
        ("x.comment-checker.json".to_owned(), 0),
    ];
    for (file_path, expected) in spelling_cases {
        let input = format!(
            r#"{{"tool_name":"Write","tool_input":{{"file_path":{},"content":"{{}}"}},"cwd":"{}"}}"#,
            serde_json::to_string(&file_path).unwrap(),
            root.display()
        );
        // when
        let result = check_hook_input(&input, "");
        // then: only the git-root config is a config edit; any other JSON stays a non-code skip
        assert_eq!(
            result.exit_code, expected,
            "{file_path}: {}",
            result.message
        );
        assert_eq!(
            result.message.contains("config-edited"),
            expected == 2,
            "{file_path}: {}",
            result.message
        );
    }
    // and: with the payload cwd inside a subdirectory, the git-root config is still the target
    let from_sub = format!(
        r#"{{"tool_name":"Write","tool_input":{{"file_path":"../.comment-checker.json","content":"{{}}"}},"cwd":"{}"}}"#,
        root.join("sub").display()
    );
    assert_eq!(check_hook_input(&from_sub, "").exit_code, 2);
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::remove_dir_all(&outside).unwrap();
}
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
fn check_hook_input_given_commented_out_code_emits_the_severity_tier() {
    // given: a proven smell on a whole-file Write
    let input = r##"{"tool_name":"Write","tool_input":{"file_path":"test.py","content":"# x = 1;\nprint(2)\n"}}"##;

    // when
    let result = check_hook_input(input, "");

    // then: the verdict names the reason and carries the tier on the comment element
    assert_eq!(result.exit_code, 2);
    assert!(
        result.message.contains("reason=\"commented-out-code\""),
        "{}",
        result.message
    );
    assert!(
        result.message.contains("severity=\"high\""),
        "{}",
        result.message
    );
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

/// A Python `Write` payload: two code lines, then `flagged` default-reason comments, then
/// `extra` verbatim lines.
fn volume_payload(flagged: usize, extra: &[&str], cwd: Option<&std::path::Path>) -> String {
    let mut content = String::from("x = 1\ny = 2\n");
    for index in 0..flagged {
        write!(
            content,
            "# narration number {index}
value_{index} = {index}
"
        )
        .unwrap();
    }
    for line in extra {
        content.push_str(line);
        content.push('\n');
    }
    let mut payload = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": "src/app.py", "content": content },
    });
    if let Some(cwd) = cwd {
        payload["cwd"] = serde_json::json!(cwd);
    }
    payload.to_string()
}

#[test]
fn check_hook_input_given_twenty_flagged_comments_reports_no_volume() {
    // given: exactly the limit of flagged comments
    let input = volume_payload(20, &[], None);

    // when
    let result = check_hook_input(&input, "");

    // then: every comment is reported on its own reason and the change is not
    assert_eq!(result.exit_code, 2);
    assert_eq!(result.message.matches("<comment ").count(), 20);
    assert!(
        !result.message.contains("comment-volume"),
        "message: {}",
        result.message
    );
}

#[test]
fn check_hook_input_given_twenty_one_flagged_comments_reports_one_volume_at_the_first() {
    // given: one comment past the limit, the first of them on line 3
    let input = volume_payload(21, &[], None);

    // when
    let result = check_hook_input(&input, "");

    // then: 21 per-comment findings plus exactly one low-severity volume finding at line 3
    assert_eq!(result.exit_code, 2);
    assert_eq!(
        result.message.matches("reason=\"comment-volume\"").count(),
        1
    );
    assert_eq!(result.message.matches("<comment ").count(), 22);
    assert!(
        result
            .message
            .contains("<comment line-number=\"3\" reason=\"comment-volume\" severity=\"low\">"),
        "message: {}",
        result.message
    );
    assert_eq!(
        result
            .message
            .matches("reason=\"unnecessary-comment\" severity=\"low\"")
            .count(),
        21,
        "per-comment reasons are unchanged: {}",
        result.message
    );
}

#[test]
fn check_hook_input_given_spared_and_marker_comments_does_not_count_them() {
    // given: 20 flagged comments plus 10 BDD comments (spared) and an invalid marker (reported
    // as allow-invalid, not as prose)
    let spared = ["# given"; 10];
    let mut extra = spared.to_vec();
    extra.push("# comment-checker: allow not-a-class -- because");
    let input = volume_payload(20, &extra, None);

    // when
    let result = check_hook_input(&input, "");

    // then: neither class inflates the count past the limit
    assert_eq!(result.exit_code, 2);
    assert!(
        result.message.contains("reason=\"allow-invalid\""),
        "message: {}",
        result.message
    );
    assert!(
        !result.message.contains("comment-volume"),
        "message: {}",
        result.message
    );
}

#[test]
fn check_hook_input_given_config_exemptions_does_not_count_them() {
    // given: 20 flagged comments plus 5 agent memos that the config exempts for the path
    let root = git_root_fixture("volume-exempt");
    std::fs::write(
        root.join(".comment-checker.json"),
        r#"{"allow":[{"glob":"src/**","reasons":["agent-memo"],"reason":"legacy notes"}]}"#,
    )
    .unwrap();
    let memos = ["# Changed from old to new"; 5];
    let input = volume_payload(20, &memos, Some(&root));

    // when
    let exempted = check_hook_input(&input, "");

    // then: the exempt memos are not counted
    assert_eq!(exempted.exit_code, 2, "{}", exempted.message);
    assert!(
        !exempted.message.contains("comment-volume"),
        "message: {}",
        exempted.message
    );

    // and: an allow entry for the volume reason drops the finding on a change past the limit
    let over = volume_payload(21, &[], Some(&root));
    assert!(
        check_hook_input(&over, "")
            .message
            .contains("comment-volume")
    );
    std::fs::write(
        root.join(".comment-checker.json"),
        r#"{"allow":[{"glob":"src/**","reasons":["comment-volume"],"reason":"generated bulk docs"}]}"#,
    )
    .unwrap();
    let allowed = check_hook_input(&over, "");
    assert_eq!(allowed.exit_code, 2, "{}", allowed.message);
    assert!(
        !allowed.message.contains("comment-volume"),
        "message: {}",
        allowed.message
    );
    std::fs::remove_dir_all(&root).unwrap();
}

fn duplicate_payload(content: &str, cwd: Option<&std::path::Path>) -> String {
    let mut payload = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": "src/app.py", "content": content },
    });
    if let Some(cwd) = cwd {
        payload["cwd"] = serde_json::json!(cwd);
    }
    payload.to_string()
}

#[test]
fn check_hook_input_given_exact_duplicate_comments_cites_the_earlier_line() {
    // given: the same comment twice, each flagged on its own
    let content = "# the retry budget is finite\nx = 1\n# the retry budget is finite\ny = 2\n";
    let input = duplicate_payload(content, None);

    // when
    let result = check_hook_input(&input, "");

    // then: one duplicate finding on the later comment cites the first one at line 1
    assert_eq!(result.exit_code, 2, "{}", result.message);
    assert_eq!(
        result
            .message
            .lines()
            .filter(|line| line.contains("reason=\"duplicate-comment\""))
            .count(),
        1,
        "{}",
        result.message
    );
    assert!(
        result
            .message
            .contains("<comment line-number=\"3\" reason=\"duplicate-comment\" severity=\"low\">"),
        "{}",
        result.message
    );
    assert!(
        !result.message.contains("near-duplicate-comment"),
        "{}",
        result.message
    );
}

#[test]
fn check_hook_input_given_short_overlap_reports_no_near_duplicate() {
    // given: comments that share a couple of words but sit under the token floor
    let content =
        "# retry budget finite\nx = 1\n# retry budget grows\n# retry budget shrinks ?\ny = 2\n";
    let input = duplicate_payload(content, None);

    // when
    let result = check_hook_input(&input, "");

    // then: no near-duplicate reason appears
    assert!(
        !result.message.contains("near-duplicate-comment"),
        "{}",
        result.message
    );
}

#[test]
fn check_hook_input_given_spared_comment_duplicating_a_flag_does_not_report() {
    // given: a flagged comment and a BDD comment with the same words (the BDD one is spared)
    let content = "# given the retry budget is finite and small\nx = 1\n// given the retry budget is finite and small\ny = 2\n";
    let input = duplicate_payload(content, None);

    // when
    let result = check_hook_input(&input, "");

    // then: the spared comment neither counts nor is cited as a duplicate
    assert!(
        !result.message.contains("duplicate-comment"),
        "{}",
        result.message
    );
    assert!(
        !result.message.contains("near-duplicate-comment"),
        "{}",
        result.message
    );
}

#[test]
fn check_hook_input_given_near_duplicates_cites_the_earlier_comment() {
    // given: two long comments sharing most tokens, with one differing word each
    let content = "# the retry budget is finite and the queue drains slowly\nx = 1\n# the retry budget is finite and the queue drains very slowly\ny = 2\n";
    let input = duplicate_payload(content, None);

    // when
    let result = check_hook_input(&input, "");

    // then: the later one is reported as a near duplicate of line 1
    assert!(
        result.message.contains(
            "<comment line-number=\"3\" reason=\"near-duplicate-comment\" severity=\"low\">"
        ),
        "{}",
        result.message
    );
    assert!(
        !result.message.contains("reason=\"duplicate-comment\""),
        "{}",
        result.message
    );
}

#[test]
fn check_hook_input_given_config_exemption_drops_the_duplicate_finding() {
    // given: an exact duplicate in a git root that exempts the duplicate reason for src/**
    let root = git_root_fixture("duplicate-exempt");
    std::fs::write(
        root.join(".comment-checker.json"),
        r#"{"allow":[{"glob":"src/**","reasons":["duplicate-comment"],"reason":"generated bulk docs"}]}"#,
    )
    .unwrap();
    let content = "# the retry budget is finite\nx = 1\n# the retry budget is finite\ny = 2\n";
    let input = duplicate_payload(content, Some(&root));

    // when
    let result = check_hook_input(&input, "");

    // then: the finding is dropped while the per-comment findings remain
    assert_eq!(result.exit_code, 2, "{}", result.message);
    assert!(
        !result.message.contains("duplicate-comment"),
        "{}",
        result.message
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn check_hook_input_given_write_baseline_reports_only_added_comments() {
    // given: a git root whose HEAD already holds the legacy file with one old comment
    let root = git_root_fixture("baseline");
    std::fs::write(root.join("app.py"), "# legacy note\nx = 1\n").unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git runs")
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.email=t@example.com",
        "-c",
        "user.name=T",
        "commit",
        "-qm",
        "base",
    ]);
    let content = "# legacy note\n# fresh note\nx = 1\n";
    let input = serde_json::json!({
        "tool_name": "Write",
        "tool_input": { "file_path": "app.py", "content": content },
        "cwd": root,
    })
    .to_string();

    // when: the opt-in baseline is on
    let with_baseline = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "printf '%s' '{}' | COMMENT_CHECKER_WRITE_BASELINE=1 {}",
            input.replace('\'', "'\\''"),
            env!("CARGO_BIN_EXE_comment-checker")
        ))
        .output()
        .expect("hook runs");
    let baseline_message = String::from_utf8_lossy(&with_baseline.stderr).into_owned();

    // then: the unchanged legacy comment is filtered out, the fresh one is reported
    assert!(
        baseline_message.contains("reason=\"unnecessary-comment\""),
        "baseline stderr: {baseline_message}"
    );
    assert!(
        baseline_message.contains("<comment line-number=\"2\""),
        "baseline stderr: {baseline_message}"
    );
    assert!(
        !baseline_message.contains("legacy note"),
        "baseline stderr: {baseline_message}"
    );

    // and: without the flag the whole file is reported again
    let without = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "printf '%s' '{}' | {}",
            input.replace('\'', "'\\''"),
            env!("CARGO_BIN_EXE_comment-checker")
        ))
        .output()
        .expect("hook runs");
    let full_message = String::from_utf8_lossy(&without.stderr);
    assert!(
        full_message.contains("legacy note"),
        "full stderr: {full_message}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

use comment_checker::detector::CommentDetector;
use comment_checker::models::CommentType;

const SMOKE_CASES: [(&str, &str, &str, CommentType); 31] = [
    (
        "main.py",
        "# Python comment\nx = 1",
        "Python comment",
        CommentType::Line,
    ),
    (
        "main.go",
        "// Go comment\npackage main",
        "Go comment",
        CommentType::Line,
    ),
    (
        "test.ts",
        "// TypeScript comment\nconst x: number = 1;",
        "TypeScript comment",
        CommentType::Line,
    ),
    (
        "test.tsx",
        "// TSX comment\nconst x = <div/>;",
        "TSX comment",
        CommentType::Line,
    ),
    (
        "test.js",
        "// JavaScript comment\nconst x = 1;",
        "JavaScript comment",
        CommentType::Line,
    ),
    (
        "Test.java",
        "// Java comment\npublic class Test {}",
        "Java comment",
        CommentType::Line,
    ),
    (
        "main.c",
        "// C comment\nint main() { return 0; }",
        "C comment",
        CommentType::Line,
    ),
    (
        "main.cpp",
        "// C++ comment\nint main() { return 0; }",
        "C++ comment",
        CommentType::Line,
    ),
    (
        "main.rs",
        "// Rust comment\nfn main() {}",
        "Rust comment",
        CommentType::Line,
    ),
    (
        "test.rb",
        "# Ruby comment\nputs \"hello\"",
        "Ruby comment",
        CommentType::Line,
    ),
    (
        "script.sh",
        "# Bash comment\necho \"hello\"",
        "Bash comment",
        CommentType::Line,
    ),
    (
        "Main.kt",
        "// Kotlin comment\nfun main() {}",
        "Kotlin comment",
        CommentType::Line,
    ),
    (
        "main.swift",
        "// Swift comment\nprint(\"hello\")",
        "Swift comment",
        CommentType::Line,
    ),
    (
        "main.cs",
        "// C# comment\nclass X {}",
        "C# comment",
        CommentType::Line,
    ),
    (
        "main.lua",
        "-- Lua comment\nlocal x = 1",
        "Lua comment",
        CommentType::Line,
    ),
    (
        "main.php",
        "<?php\n// PHP comment\n$x = 1;",
        "PHP comment",
        CommentType::Line,
    ),
    (
        "main.ex",
        "# Elixir comment\ndefmodule X do\nend",
        "Elixir comment",
        CommentType::Line,
    ),
    (
        "main.ml",
        "(* OCaml comment *)\nlet x = 1",
        "OCaml comment",
        CommentType::Block,
    ),
    (
        "Main.scala",
        "// Scala comment\nobject X {}",
        "Scala comment",
        CommentType::Line,
    ),
    (
        "build.groovy",
        "// Groovy comment\ndef x = 1",
        "Groovy comment",
        CommentType::Line,
    ),
    (
        "main.proto",
        "// Proto comment\nmessage X {}",
        "Proto comment",
        CommentType::Line,
    ),
    (
        "config.hcl",
        "# HCL comment\nresource \"x\" \"y\" {}",
        "HCL comment",
        CommentType::Line,
    ),
    (
        "main.sql",
        "-- SQL comment\nSELECT 1;",
        "SQL comment",
        CommentType::Line,
    ),
    (
        "config.toml",
        "# TOML comment\nx = 1",
        "TOML comment",
        CommentType::Line,
    ),
    (
        "config.yaml",
        "# YAML comment\nx: 1",
        "YAML comment",
        CommentType::Line,
    ),
    (
        "Main.elm",
        "-- Elm comment\nmodule Main exposing (..)",
        "Elm comment",
        CommentType::Line,
    ),
    (
        "config.cue",
        "// CUE comment\npackage x",
        "CUE comment",
        CommentType::Line,
    ),
    (
        "Dockerfile",
        "# Dockerfile comment\nFROM debian",
        "Dockerfile comment",
        CommentType::Line,
    ),
    (
        "index.html",
        "<!-- HTML comment -->\n<html></html>",
        "HTML comment",
        CommentType::Block,
    ),
    (
        "style.css",
        "/* CSS comment */\nbody { color: red; }",
        "CSS comment",
        CommentType::Block,
    ),
    (
        "App.svelte",
        "<!-- Svelte comment -->\n<div></div>",
        "Svelte comment",
        CommentType::Block,
    ),
];

#[test]
fn detect_given_python_line_comment_returns_comment_info() {
    // given
    let detector = CommentDetector::new();
    let code = "# This is a comment\nprint('hello')";

    // when
    let comments = detector.detect(code, "test.py", false);

    // then
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text, "# This is a comment");
    assert_eq!(comments[0].line_number, 1);
    assert_eq!(comments[0].file_path, "test.py");
    assert_eq!(comments[0].comment_type, CommentType::Line);
    assert!(!comments[0].is_docstring);
}

#[test]
fn detect_given_typescript_block_comment_returns_block_comment() {
    // given
    let detector = CommentDetector::new();
    let code = "/* This is a block comment */\nconst x = 1;";

    // when
    let comments = detector.detect(code, "test.ts", false);

    // then
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text, "/* This is a block comment */");
    assert_eq!(comments[0].comment_type, CommentType::Block);
}

#[test]
fn detect_given_python_docstring_with_docstrings_returns_docstring() {
    // given
    let detector = CommentDetector::new();
    let code = "\"\"\"This is a module docstring.\"\"\"\ndef hello():\n    pass";

    // when
    let comments = detector.detect(code, "module.py", true);

    // then
    assert!(comments.iter().any(|comment| {
        comment.is_docstring
            && comment.comment_type == CommentType::Docstring
            && comment.text.contains("module docstring")
    }));
}

#[test]
fn detect_given_unsupported_extension_returns_empty_list() {
    // given
    let detector = CommentDetector::new();
    let code = "# This is a comment\n// and another\nprint('hello')";

    // when
    let supported = detector.detect(code, "test.py", false);
    let comments = detector.detect(code, "test.xyz", false);

    // then: the same comment-bearing bytes yield comments for a supported extension, none for an unsupported one
    assert_eq!(supported.len(), 1);
    assert_eq!(comments.len(), 0);
}

#[test]
fn detect_given_go_comment_returns_comment_info() {
    // given
    let detector = CommentDetector::new();
    let code = "// This is a Go comment\npackage main\n\nfunc main() {}";

    // when
    let comments = detector.detect(code, "main.go", false);

    // then
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text, "// This is a Go comment");
    assert_eq!(comments[0].line_number, 1);
    assert_eq!(comments[0].file_path, "main.go");
    assert_eq!(comments[0].comment_type, CommentType::Line);
    assert!(!comments[0].is_docstring);
}

#[test]
fn detect_given_multilanguage_smoke_cases_returns_comment() {
    // given
    let detector = CommentDetector::new();

    // when
    let failures: Vec<String> = SMOKE_CASES
        .iter()
        .filter_map(|&(path, source, expected, expected_type)| {
            let comments = detector.detect(source, path, false);
            let matched = comments.len() == 1
                && comments[0].text.contains(expected)
                && comments[0].comment_type == expected_type;
            (!matched).then(|| {
                format!(
                    "{expected}: {found:?}",
                    found = comments
                        .iter()
                        .map(|comment| (comment.text.as_str(), comment.comment_type))
                        .collect::<Vec<_>>()
                )
            })
        })
        .collect();

    // then
    assert_eq!(failures.len(), 0, "smoke failures: {failures:#?}");
}

#[test]
fn detect_given_javascript_block_comment_returns_block_comment() {
    // given
    let detector = CommentDetector::new();
    let code = "/* block comment */\nconst x = 1;";

    // when
    let comments = detector.detect(code, "test.js", false);

    // then
    assert_eq!(comments.len(), 1);
    assert!(comments[0].text.contains("block comment"));
    assert_eq!(comments[0].comment_type, CommentType::Block);
}

#[test]
fn detect_given_c_block_comment_returns_block_comment() {
    // given
    let detector = CommentDetector::new();
    let code = "/* C block comment */\nint main() { return 0; }";

    // when
    let comments = detector.detect(code, "main.c", false);

    // then
    assert_eq!(comments.len(), 1);
    assert!(comments[0].text.contains("C block comment"));
    assert_eq!(comments[0].comment_type, CommentType::Block);
}

#[test]
fn detect_given_javascript_line_comment_with_docstrings_is_not_a_docstring() {
    // given
    let detector = CommentDetector::new();
    let code = "// regular comment\nconst x = 1;";

    // when
    let comments = detector.detect(code, "test.js", true);

    // then
    assert_eq!(comments.len(), 1);
    assert!(!comments[0].is_docstring);
}

#[test]
fn detect_given_comment_inside_html_script_element_is_detected() {
    // given
    let detector = CommentDetector::new();
    let code =
        "<html><body>\n<script>\n// injected comment\nconst x = 1;\n</script>\n</body></html>";

    // when
    let comments = detector.detect(code, "page.html", true);

    // then
    assert!(
        comments
            .iter()
            .any(|comment| comment.text.contains("injected comment"))
    );
}

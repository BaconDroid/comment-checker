use comment_checker::api::check_hook_input;
use comment_checker::detector::CommentDetector;
use comment_checker::filters::AgentMemoFilter;
use comment_checker::models::{CommentInfo, CommentType};
use comment_checker::output::{build_comments_xml, format_hook_message};
use std::hint::black_box;
use std::time::Instant;

const WARMUP_ITERATIONS: u32 = 100;
const MEASURED_ITERATIONS: u32 = 1_000;

const PYTHON_SOURCE: &str = "\"\"\"Module docstring.\"\"\"\n\n# regular comment\ndef add(a, b):\n    \"\"\"Function docstring.\"\"\"\n    return a + b\n";

fn measure(name: &str, run: &mut impl FnMut()) {
    for _ in 0..WARMUP_ITERATIONS {
        run();
    }
    let start = Instant::now();
    for _ in 0..MEASURED_ITERATIONS {
        run();
    }
    println!(
        "{name}: {:?} per iteration",
        start.elapsed() / MEASURED_ITERATIONS
    );
}

fn main() {
    let detector = CommentDetector::new();
    measure("detect_python_no_docstrings", &mut || {
        black_box(detector.detect(black_box(PYTHON_SOURCE), black_box("sample.py"), false));
    });
    measure("detect_python_with_docstrings", &mut || {
        black_box(detector.detect(black_box(PYTHON_SOURCE), black_box("sample.py"), true));
    });
    measure("detect_unsupported_extension", &mut || {
        black_box(detector.detect(black_box(PYTHON_SOURCE), black_box("sample.unknown"), true));
    });

    let filter = AgentMemoFilter;
    let early_hit = CommentInfo::new(
        "// Changed from old to new",
        1,
        "sample.py",
        CommentType::Line,
        false,
    );
    let late_hit = CommentInfo::new(
        "// Switched from old to new",
        1,
        "sample.py",
        CommentType::Line,
        false,
    );
    let full_miss = CommentInfo::new(
        "// calculate checksum before write",
        1,
        "sample.py",
        CommentType::Line,
        false,
    );
    measure("agent_memo_early_hit", &mut || {
        black_box(filter.is_agent_memo(black_box(&early_hit)));
    });
    measure("agent_memo_late_hit", &mut || {
        black_box(filter.is_agent_memo(black_box(&late_hit)));
    });
    measure("agent_memo_full_miss", &mut || {
        black_box(filter.is_agent_memo(black_box(&full_miss)));
    });

    let comments: Vec<CommentInfo> = (0..200)
        .map(|index| {
            CommentInfo::new(
                format!("# comment {index}"),
                index + 1,
                "sample.py",
                CommentType::Line,
                false,
            )
        })
        .collect();
    measure("build_comments_xml_many_comments", &mut || {
        black_box(build_comments_xml(
            black_box(&comments),
            black_box("sample.py"),
        ));
    });
    measure("format_hook_message_many_comments", &mut || {
        black_box(format_hook_message(black_box(&comments), black_box("")));
    });

    let edit_input = r##"{"tool_name":"Edit","tool_input":{"file_path":"sample.py","old_string":"print(1)","new_string":"# new comment\nprint(1)"}}"##;
    let multiedit_input = r##"{"tool_name":"MultiEdit","tool_input":{"file_path":"sample.py","edits":[{"old_string":"print(1)","new_string":"# new comment\nprint(1)"},{"old_string":"print(2)","new_string":"# second comment\nprint(2)"}]}}"##;
    measure("check_hook_input_edit_new_comment", &mut || {
        black_box(check_hook_input(
            black_box(edit_input),
            black_box("{{comments}}"),
        ));
    });
    measure("check_hook_input_multiedit_new_comments", &mut || {
        black_box(check_hook_input(
            black_box(multiedit_input),
            black_box("{{comments}}"),
        ));
    });
}
